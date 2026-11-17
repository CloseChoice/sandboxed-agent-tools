# Install the Windows x64 SAT host and optional default-tools or python,jaq.
param(
    [Parameter(Position = 0)][string]$Tools = "",
    [string]$Version = "latest",
    [ValidateSet("windows-x64")][string]$Platform = "windows-x64",
    [string]$BinDir = (Join-Path $env:LOCALAPPDATA "SAT\bin")
)
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$repo = "https://github.com/CloseChoice/sandboxed-agent-tools"
$target = "x86_64-pc-windows-msvc"
$selected = @()
if ($Tools -eq "default-tools") {
    $selected = @("python", "jaq", "code", "markitdown", "ripgrep")
} elseif ($Tools) {
    $selected = @($Tools.Split(","))
}
foreach ($tool in $selected) {
    if ($tool -notin @("python", "jaq", "code", "markitdown", "ripgrep")) {
        throw "Unknown tool: $tool"
    }
}
if ($Version -eq "latest") {
    $Version = (Invoke-RestMethod "https://api.github.com/repos/CloseChoice/sandboxed-agent-tools/releases/latest").tag_name
}
$Version = $Version -replace "^v", ""
if ($Version -cnotmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$') {
    throw "Invalid version: $Version"
}
$base = "$repo/releases/download/v$Version"
$asset = "sat-$Version-$target.zip"
$satWorkDir = Join-Path ([IO.Path]::GetTempPath()) ("sat-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $satWorkDir | Out-Null
try {
    Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS" -OutFile (Join-Path $satWorkDir "SHA256SUMS")
    function Get-ReleaseAsset([string]$Name) {
        $path = Join-Path $satWorkDir $Name
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$Name" -OutFile $path
        $pattern = '^[0-9a-fA-F]{64}\s+' + [regex]::Escape($Name) + '$'
        $entry = @(Select-String -LiteralPath (Join-Path $satWorkDir "SHA256SUMS") -Pattern $pattern)
        if ($entry.Count -ne 1) { throw "Missing or duplicate checksum for $Name" }
        $expected = ($entry[0].Line -split '\s+')[0]
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ine $expected) {
            throw "Checksum mismatch for $Name"
        }
    }
    # Verify everything before replacing or executing the host.
    Get-ReleaseAsset $asset
    foreach ($tool in $selected) { Get-ReleaseAsset "$tool.tar" }
    Expand-Archive -LiteralPath (Join-Path $satWorkDir $asset) -DestinationPath $satWorkDir
    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    $satPath = Join-Path (Resolve-Path -LiteralPath $BinDir).Path "sat.exe"
    Move-Item -Force -LiteralPath (Join-Path $satWorkDir "sat.exe") -Destination $satPath
    Push-Location $satWorkDir
    try {
        foreach ($tool in $selected) {
            # Supply empty permissions; do not change the caller's project config.
            @("[]", "[]", "[]", "[]", "[]", "unset") |
                & $satPath register --force (Join-Path $satWorkDir "$tool.tar") | Out-Null
            if ($LASTEXITCODE -ne 0) { throw "Registration failed for $tool" }
            Write-Host "Installed tool: $tool"
        }
    } finally { Pop-Location }
    Write-Host "Installed SAT $Version to $satPath"
    Write-Host ('Add to PATH if needed: $env:Path = "' + (Split-Path $satPath) + ';$env:Path"')
} finally {
    Remove-Item -LiteralPath $satWorkDir -Recurse -Force
}
