param([switch]$SkipBuild, [string]$Executable)
$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $taskRoot
try {
    $binary = if ($Executable) { [System.IO.Path]::GetFullPath($Executable) } else { Join-Path $taskRoot 'target/release/vibecolor.exe' }
    if (-not $SkipBuild) {
        # Emit a fresh binary so active MCP sessions can keep their executable.
        $binary = Join-Path $taskRoot ('target/release/vibecolor-plugin-' + [guid]::NewGuid().ToString('N') + '.exe')
        & cargo rustc --release --locked -p vibecolor-cli --bin vibecolor -- "--emit=link=$binary"
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    }
    if (-not (Test-Path -LiteralPath $binary)) { throw 'Build the Windows release binary first' }
    # A fresh package avoids retaining stale files from an earlier version.
    $packageRoot = Join-Path $taskRoot ('target/plugin-' + [guid]::NewGuid().ToString('N'))
    $package = Join-Path $packageRoot 'tinge'
    New-Item -ItemType Directory -Path $package | Out-Null
    Get-ChildItem -LiteralPath (Join-Path $taskRoot 'plugin') -Force | Copy-Item -Destination $package -Recurse
    $binDirectory = Join-Path $package 'bin'
    New-Item -ItemType Directory -Path $binDirectory | Out-Null
    Copy-Item -LiteralPath $binary -Destination (Join-Path $binDirectory 'vibecolor.exe')
    foreach ($name in @('LICENSE', 'THIRD_PARTY.md')) {
        Copy-Item -LiteralPath (Join-Path $taskRoot $name) -Destination $package
    }
    $referenceDirectory = Join-Path $package 'skills/photo-workflow/references'
    New-Item -ItemType Directory -Path $referenceDirectory | Out-Null
    Get-ChildItem -LiteralPath (Join-Path $taskRoot 'docs') -Filter '*.md' | Copy-Item -Destination $referenceDirectory
    $marketplaceDirectory = Join-Path $packageRoot '.agents/plugins'
    New-Item -ItemType Directory -Path $marketplaceDirectory -Force | Out-Null
    @{
        name = 'tinge-local'
        interface = @{ displayName = 'Tinge Local' }
        plugins = @(@{
            name = 'tinge'
            source = @{ source = 'local'; path = './tinge' }
            policy = @{ installation = 'AVAILABLE'; authentication = 'ON_USE' }
            category = 'Productivity'
        })
    } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $marketplaceDirectory 'marketplace.json') -Encoding utf8NoBOM
    Write-Output $packageRoot
} finally {
    Pop-Location
}
