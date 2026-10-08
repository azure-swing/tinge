param([Parameter(Mandatory)][string]$Directory, [switch]$Restore)
$ErrorActionPreference = 'Stop'
$fixtureDirectory = [System.IO.Path]::GetFullPath($Directory)
$fixtureName = [System.IO.Path]::GetFileName($fixtureDirectory)
if (-not $fixtureName.StartsWith('vibecolor-storage-')) { throw 'Only named acceptance fixture directories are supported' }
$shell = New-Object -ComObject Shell.Application
$items = @($shell.Namespace(10).Items())
$found = @()
foreach ($item in $items) {
    $originalDirectory = [string]$item.ExtendedProperty('System.Recycle.DeletedFrom')
    if ($originalDirectory.TrimEnd('\') -eq $fixtureDirectory.TrimEnd('\')) {
        $stream = [System.IO.File]::OpenRead($item.Path)
        $hasher = [System.Security.Cryptography.SHA256]::Create()
        try { $digest = [System.BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-','') }
        finally { $stream.Dispose(); $hasher.Dispose() }
        $found += [ordered]@{ name=$item.Name; sha256=$digest; recycled_path=$item.Path }
        if ($Restore) { $item.InvokeVerb('undelete') }
    }
}
ConvertTo-Json -InputObject $found -Depth 4 -Compress
