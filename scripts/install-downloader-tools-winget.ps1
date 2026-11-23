<#
.SYNOPSIS
    Installs or updates yt-dlp and FFmpeg with WinGet.

.DESCRIPTION
    Used by RollaPlay's Downloader when the user ticks the tools to install.
    Only the official WinGet package IDs are used. Output is filtered so no
    URLs, paths or credentials can reach the app's diagnostics.
#>
param(
    [switch] $InstallYtDlp,
    [switch] $InstallFfmpeg
)

$ErrorActionPreference = 'Stop'

function Find-Winget {
    $cmd = Get-Command winget.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\winget.exe'),
        (Join-Path $env:ProgramFiles 'WindowsApps\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe\winget.exe')
    )
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    return $null
}

function Write-SafeLine {
    param([string] $Line)
    $text = $Line.Trim()
    if ([string]::IsNullOrWhiteSpace($text)) { return }
    if ($text -match '(?i)https?://|[A-Za-z]:\\|\\\\|\b(token|bearer|password|credential)\b') { return }
    if ($text.Length -gt 120) { $text = $text.Substring(0, 120) }
    Write-Output $text
}

function Ensure-Package {
    param([string] $Id, [string] $Label)
    Write-SafeLine "Installing $Label with WinGet"
    $script:installNote = ''
    & $winget install --id $Id --exact --source winget --accept-source-agreements --accept-package-agreements --disable-interactivity 2>&1 | ForEach-Object {
        $line = [string]$_
        if ($line -match '(?i)already installed|no available upgrade|no newer package') { $script:installNote = 'current' }
        Write-SafeLine $line
    }
    $code = $LASTEXITCODE
    # WinGet returns a negative "already installed / no upgrade" code when the
    # newest version is present. Treat that as success, never as a failure.
    if ($code -ne 0 -and $script:installNote -eq '') {
        throw "WinGet could not install $Label"
    }
    Write-SafeLine "$Label is ready"
}

$winget = Find-Winget
if (-not $winget) {
    Write-SafeLine 'WinGet is not available on this Windows installation.'
    exit 1
}

if ($InstallYtDlp) { Ensure-Package 'yt-dlp.yt-dlp' 'yt-dlp' }
if ($InstallFfmpeg) { Ensure-Package 'yt-dlp.FFmpeg' 'FFmpeg' }

Write-SafeLine 'TOOLS_READY'
