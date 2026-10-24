# Installs brok and brok-agent from GitHub Releases (Windows).
#
#   irm https://raw.githubusercontent.com/brokspace/brok/main/scripts/install.ps1 | iex
#
# Environment overrides:
#   BROK_VERSION      release tag to install (default: latest)
#   BROK_INSTALL_DIR  destination directory (default: %LOCALAPPDATA%\Programs\brok)
#   BROK_NO_MODIFY_PATH=1  do not add the directory to the user PATH
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Version = if ($env:BROK_VERSION) { $env:BROK_VERSION } else { 'latest' }
$InstallDir = if ($env:BROK_INSTALL_DIR) { $env:BROK_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\brok' }
$Base = 'https://github.com/brokspace'

$Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($Arch -ne 'X64') { throw "Unsupported architecture $($Arch): only x86_64 Windows builds are published today." }
$Platform = 'windows-x86_64'

New-Item -ItemType Directory -Force $InstallDir | Out-Null
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("brok-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $Tmp | Out-Null

try {
    foreach ($Bin in @('brok', 'brok-agent')) {
        $Asset = "$Bin-$Platform.exe"
        $Url = if ($Version -eq 'latest') { "$Base/$Bin/releases/latest/download/$Asset" } else { "$Base/$Bin/releases/download/$Version/$Asset" }
        Write-Host "- downloading $Bin ($Platform)"
        $File = Join-Path $Tmp $Asset
        Invoke-WebRequest -Uri $Url -OutFile $File -UseBasicParsing
        Invoke-WebRequest -Uri "$Url.sha256" -OutFile "$File.sha256" -UseBasicParsing
        $Expected = ((Get-Content "$File.sha256" -Raw).Trim() -split '\s+')[0].ToLower()
        $Actual = (Get-FileHash $File -Algorithm SHA256).Hash.ToLower()
        if ($Expected -ne $Actual) { throw "Checksum mismatch for $Bin (expected $Expected, got $Actual)" }
        Move-Item -Force $File (Join-Path $InstallDir "$Bin.exe")
    }
}
finally {
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}

$UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($env:BROK_NO_MODIFY_PATH) {
    Write-Host "- PATH left unchanged (BROK_NO_MODIFY_PATH); add $InstallDir yourself"
}
elseif (-not (($UserPath -split ';') -contains $InstallDir)) {
    [Environment]::SetEnvironmentVariable('Path', "$UserPath;$InstallDir", 'User')
    $env:Path = "$env:Path;$InstallDir"
    Write-Host "- added $InstallDir to your user PATH (new terminals pick it up)"
}

Write-Host "OK: installed brok and brok-agent into $InstallDir"
Write-Host ""
Write-Host "Next, in your project:  brok init"
