[CmdletBinding()]
param(
    [string]$ArchivePath = "",
    [string]$Version = "0.145.0-beta.1"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Installer = Join-Path $PSScriptRoot "install.ps1"
$Architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
$Target = switch ($Architecture) {
    "X64" { "x86_64-pc-windows-msvc" }
    "Arm64" { "aarch64-pc-windows-msvc" }
    default { throw "Unsupported Windows architecture: $Architecture" }
}
$AssetName = "codex-quiet-$Version-$Target.zip"
$Root = Join-Path ([System.IO.Path]::GetTempPath()) "quiet-installer-test.$PID.$([Guid]::NewGuid().ToString('N'))"
$Fixtures = Join-Path $Root "fixtures"
$Package = Join-Path $Root "package"
$InstallRoot = Join-Path $Root "install-$([char]0x4F8B)-$([char]0x00E9)"
$BinDir = Join-Path $InstallRoot "bin"
$FixtureArchive = Join-Path $Fixtures "asset.zip"
$SumsPath = Join-Path $Fixtures "SHA256SUMS"
$OriginalProcessPath = $env:Path
$OriginalUserPath = [Environment]::GetEnvironmentVariable("Path", "User")
$OriginalRelease = $env:CODEX_QUIET_RELEASE
$global:QuietInstallerTestUrls = [System.Collections.Generic.List[string]]::new()
$global:QuietInstallerTestFixtures = $Fixtures
$global:QuietInstallerTestVersion = $Version

function global:Invoke-WebRequest {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [uri]$Uri,

        [Parameter(Mandatory = $true)]
        [string]$OutFile,

        [switch]$UseBasicParsing
    )

    $global:QuietInstallerTestUrls.Add($Uri.AbsoluteUri)
    if ($Uri.AbsolutePath.EndsWith("/SHA256SUMS")) {
        Copy-Item -LiteralPath (Join-Path $global:QuietInstallerTestFixtures "SHA256SUMS") -Destination $OutFile
        return
    }
    if ($Uri.AbsolutePath.EndsWith(".zip")) {
        Copy-Item -LiteralPath (Join-Path $global:QuietInstallerTestFixtures "asset.zip") -Destination $OutFile
        return
    }
    throw "Unexpected fixture URL: $Uri"
}

function global:Invoke-RestMethod {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)]
        [uri]$Uri,

        [hashtable]$Headers,

        [switch]$UseBasicParsing
    )

    $global:QuietInstallerTestUrls.Add($Uri.AbsoluteUri)
    if ($Uri.AbsoluteUri -cne "https://api.github.com/repos/maherr/quiet-for-codex/releases?per_page=20") {
        throw "Unexpected fixture API URL: $Uri"
    }
    return @(
        [pscustomobject]@{ tag_name = "v999.0.0"; prerelease = $false },
        [pscustomobject]@{ tag_name = "quiet-v$global:QuietInstallerTestVersion"; prerelease = $true }
    )
}

try {
    New-Item -ItemType Directory -Force -Path $Fixtures | Out-Null
    if ($ArchivePath) {
        $ResolvedArchive = Resolve-Path $ArchivePath
        if ((Split-Path -Leaf $ResolvedArchive) -cne $AssetName) {
            throw "Expected release archive $AssetName, got $(Split-Path -Leaf $ResolvedArchive)"
        }
        Copy-Item -LiteralPath $ResolvedArchive -Destination $FixtureArchive
    } else {
        $PackageBin = Join-Path $Package "bin"
        New-Item -ItemType Directory -Force -Path $PackageBin | Out-Null
        Copy-Item -LiteralPath $env:ComSpec -Destination (Join-Path $PackageBin "codex-quiet.exe")
        Copy-Item -LiteralPath $env:ComSpec -Destination (Join-Path $PackageBin "codex-code-mode-host.exe")
        Compress-Archive -Path (Join-Path $Package "*") -DestinationPath $FixtureArchive
    }

    $Digest = (Get-FileHash -LiteralPath $FixtureArchive -Algorithm SHA256).Hash.ToLowerInvariant()
    Set-Content -LiteralPath $SumsPath -Encoding Ascii -Value "$Digest  $AssetName"

    $PathPrefix = "$BinDir;"
    $env:Path = "$PathPrefix$OriginalProcessPath"
    $env:CODEX_QUIET_RELEASE = $null
    [Environment]::SetEnvironmentVariable("Path", "$PathPrefix$OriginalUserPath", "User")

    & {
        . $Installer -InstallRoot $InstallRoot
    }
    & {
        . $Installer -Release $Version -InstallRoot $InstallRoot
    }

    $InstalledExe = Join-Path $InstallRoot "releases\$Version-$Target\bin\codex-quiet.exe"
    $InstalledHost = Join-Path $InstallRoot "releases\$Version-$Target\bin\codex-code-mode-host.exe"
    $Shim = Join-Path $BinDir "codex-quiet.cmd"
    foreach ($RequiredPath in @($InstalledExe, $InstalledHost, $Shim)) {
        if (-not (Test-Path -LiteralPath $RequiredPath -PathType Leaf)) {
            throw "Installer did not create $RequiredPath"
        }
    }
    $ExpectedShim = "@`"%~dp0..\releases\$Version-$Target\bin\codex-quiet.exe`" %*"
    $ActualShim = (Get-Content -LiteralPath $Shim -Raw).TrimEnd([char[]]"`r`n")
    if ($ActualShim -cne $ExpectedShim) {
        throw "Installer shim is not the expected root-relative command: $ActualShim"
    }
    if ($ActualShim.Contains($InstallRoot)) {
        throw "Installer shim embeds the absolute installation root."
    }
    $CurrentTarget = (Get-Content -LiteralPath (Join-Path $InstallRoot "current.txt") -Raw).Trim()
    $ExpectedCurrentTarget = Join-Path $InstallRoot "releases\$Version-$Target"
    if ($CurrentTarget -cne $ExpectedCurrentTarget) {
        throw "current.txt did not preserve the Unicode installation path."
    }

    if ($ArchivePath) {
        $VersionOutput = & $Shim --version 2>&1
        if ($LASTEXITCODE -ne 0 -or ([string]::Join("`n", [string[]]$VersionOutput)) -notmatch "(?i)codex") {
            throw "Installed release shim failed its version smoke test."
        }
        $HelpOutput = & $Shim --help 2>&1
        if ($LASTEXITCODE -ne 0 -or ([string]::Join("`n", [string[]]$HelpOutput)) -notmatch "(?i)usage") {
            throw "Installed release shim failed to forward the help argument."
        }
    } else {
        $ProbeOutput = & $Shim /d /c "echo SHIM_ARGUMENT_OK" 2>&1
        if ($LASTEXITCODE -ne 0 -or ([string]::Join("`n", [string[]]$ProbeOutput)) -notmatch "SHIM_ARGUMENT_OK") {
            throw "Synthetic installer shim failed to resolve or forward arguments."
        }
    }

    $ExpectedBase = "https://github.com/maherr/quiet-for-codex/releases/download/quiet-v$Version"
    $ExpectedUrls = @(
        "https://api.github.com/repos/maherr/quiet-for-codex/releases?per_page=20",
        "$ExpectedBase/$AssetName",
        "$ExpectedBase/SHA256SUMS",
        "$ExpectedBase/$AssetName",
        "$ExpectedBase/SHA256SUMS"
    )
    $ActualUrlText = [string]::Join("`n", [string[]]$global:QuietInstallerTestUrls)
    $ExpectedUrlText = [string]::Join("`n", [string[]]$ExpectedUrls)
    if ($ActualUrlText -cne $ExpectedUrlText) {
        throw "Installer requested an unexpected URL set: $ActualUrlText"
    }
} finally {
    $env:Path = $OriginalProcessPath
    $env:CODEX_QUIET_RELEASE = $OriginalRelease
    [Environment]::SetEnvironmentVariable("Path", $OriginalUserPath, "User")
    Remove-Item Function:\global:Invoke-WebRequest -ErrorAction SilentlyContinue
    Remove-Item Function:\global:Invoke-RestMethod -ErrorAction SilentlyContinue
    Remove-Variable QuietInstallerTestUrls -Scope Global -ErrorAction SilentlyContinue
    Remove-Variable QuietInstallerTestFixtures -Scope Global -ErrorAction SilentlyContinue
    Remove-Variable QuietInstallerTestVersion -Scope Global -ErrorAction SilentlyContinue
    if (Test-Path -LiteralPath $Root) {
        Remove-Item -LiteralPath $Root -Recurse -Force
    }
}

Write-Host "PowerShell installer smoke passed for $Target."
