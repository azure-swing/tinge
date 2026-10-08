param([string]$Executable)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not $Executable) { $Executable = Join-Path $projectRoot 'target/release/vibecolor.exe' }
foreach ($kind in @('request', 'recipe', 'edits')) {
    $result = & $Executable schema $kind
    if ($LASTEXITCODE -ne 0) { throw "schema $kind failed" }
    $envelope = $result | ConvertFrom-Json
    if (-not $envelope.ok) { throw "schema $kind returned an error" }
    $content = ($envelope.data | ConvertTo-Json -Depth 100) + "`n"
    [System.IO.File]::WriteAllText((Join-Path $projectRoot "schemas/$kind.schema.json"), $content, [System.Text.UTF8Encoding]::new($false))
}
