[CmdletBinding()]
param(
    [string]$Release = $env:CODEX_QUIET_RELEASE,
    [string]$InstallRoot = $env:CODEX_QUIET_INSTALL_ROOT
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Repository = "maherr/quiet-for-codex"
if ([string]::IsNullOrWhiteSpace($Release)) {
    $Release = "latest"
}
if ([string]::IsNullOrWhiteSpace($InstallRoot)) {
    $InstallRoot = Join-Path $env:LOCALAPPDATA "CodexQuiet"
}

function Write-Step {
    param([string]$Message)
    Write-Host "==> $Message"
}

function Normalize-Version {
    param([string]$Value)
    if ($Value.StartsWith("quiet-v")) {
        return $Value.Substring(7)
    }
    if ($Value.StartsWith("v")) {
        return $Value.Substring(1)
    }
    return $Value
}

function Assert-Version {
    param([string]$Value)
    if ($Value -cnotmatch "^[0-9]+\.[0-9]+\.[0-9]+(?:-(?:alpha|beta|rc)(?:\.[0-9]+)?)?$") {
        throw "Invalid release version: $Value"
    }
}

function Resolve-Version {
    if ($Release -ne "latest") {
        return Normalize-Version -Value $Release
    }

    $headers = @{ "User-Agent" = "codex-quiet-installer" }
    $releases = Invoke-RestMethod -UseBasicParsing -Headers $headers -Uri "https://api.github.com/repos/$Repository/releases?per_page=20"
    $match = $releases |
        Where-Object { $_.tag_name -match "^quiet-v[0-9]" } |
        Select-Object -First 1
    if ($null -eq $match) {
        throw "No Quiet for Codex release was found."
    }
    return Normalize-Version -Value ([string]$match.tag_name)
}

function Resolve-Target {
    $architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
    switch ($architecture) {
        "X64" { return "x86_64-pc-windows-msvc" }
        "Arm64" { return "aarch64-pc-windows-msvc" }
        default { throw "Unsupported Windows architecture: $architecture" }
    }
}

function Path-Contains {
    param([string]$PathValue, [string]$Entry)
    if ([string]::IsNullOrWhiteSpace($PathValue)) {
        return $false
    }
    foreach ($segment in $PathValue.Split(";", [System.StringSplitOptions]::RemoveEmptyEntries)) {
        if ($segment.TrimEnd("\") -ieq $Entry.TrimEnd("\")) {
            return $true
        }
    }
    return $false
}

$Version = Resolve-Version
Assert-Version -Value $Version
$Target = Resolve-Target
$Tag = "quiet-v$Version"
$Asset = "codex-quiet-$Version-$Target.zip"
$BaseUrl = "https://github.com/$Repository/releases/download/$Tag"

$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) "codex-quiet-install.$PID"
$ArchivePath = Join-Path $TempDir $Asset
$SumsPath = Join-Path $TempDir "SHA256SUMS"

try {
    New-Item -ItemType Directory -Force -Path $TempDir | Out-Null
    Write-Step "Downloading Quiet for Codex $Version for $Target"
    Invoke-WebRequest -UseBasicParsing -Uri "$BaseUrl/$Asset" -OutFile $ArchivePath
    Invoke-WebRequest -UseBasicParsing -Uri "$BaseUrl/SHA256SUMS" -OutFile $SumsPath

    $escapedAsset = [regex]::Escape($Asset)
    $expected = $null
    foreach ($line in Get-Content -LiteralPath $SumsPath) {
        if ($line -match "^([0-9a-fA-F]{64})\s+$escapedAsset$") {
            $expected = $matches[1].ToLowerInvariant()
            break
        }
    }
    if ([string]::IsNullOrWhiteSpace($expected)) {
        throw "SHA256SUMS has no digest for $Asset."
    }
    $actual = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($expected -ne $actual) {
        throw "Archive checksum mismatch."
    }

    $ReleasesDir = Join-Path $InstallRoot "releases"
    $TargetDir = Join-Path $ReleasesDir "$Version-$Target"
    $StagingDir = Join-Path $ReleasesDir ".staging.$Version.$PID"
    New-Item -ItemType Directory -Force -Path $ReleasesDir | Out-Null
    if (Test-Path -LiteralPath $StagingDir) {
        Remove-Item -LiteralPath $StagingDir -Recurse -Force
    }
    Expand-Archive -LiteralPath $ArchivePath -DestinationPath $StagingDir

    $QuietExe = Join-Path $StagingDir "bin\codex-quiet.exe"
    $HostExe = Join-Path $StagingDir "bin\codex-code-mode-host.exe"
    if (-not (Test-Path -LiteralPath $QuietExe -PathType Leaf)) {
        throw "Archive is missing bin\codex-quiet.exe."
    }
    if (-not (Test-Path -LiteralPath $HostExe -PathType Leaf)) {
        throw "Archive is missing bin\codex-code-mode-host.exe."
    }

    if (Test-Path -LiteralPath $TargetDir) {
        Remove-Item -LiteralPath $TargetDir -Recurse -Force
    }
    Move-Item -LiteralPath $StagingDir -Destination $TargetDir

    $BinDir = Join-Path $InstallRoot "bin"
    $ShimPath = Join-Path $BinDir "codex-quiet.cmd"
    $ShimTarget = "%~dp0..\releases\$Version-$Target\bin\codex-quiet.exe"
    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    Set-Content -LiteralPath $ShimPath -Encoding Ascii -Value "@`"$ShimTarget`" %*"
    Set-Content -LiteralPath (Join-Path $InstallRoot "current.txt") -Encoding utf8 -Value $TargetDir

    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (-not (Path-Contains -PathValue $UserPath -Entry $BinDir)) {
        $NewUserPath = if ([string]::IsNullOrWhiteSpace($UserPath)) {
            $BinDir
        } else {
            "$BinDir;$UserPath"
        }
        [Environment]::SetEnvironmentVariable("Path", $NewUserPath, "User")
    }
    if (-not (Path-Contains -PathValue $env:Path -Entry $BinDir)) {
        $env:Path = "$BinDir;$env:Path"
    }

    Write-Step "Installed $ShimPath"
    Write-Host "Open a new terminal, then run codex-quiet."
} finally {
    if (Test-Path -LiteralPath $TempDir) {
        Remove-Item -LiteralPath $TempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
