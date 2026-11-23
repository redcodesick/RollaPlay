<#
.SYNOPSIS
    Downloads the official yt-dlp and FFmpeg binaries from their GitHub releases.

.DESCRIPTION
    Used by RollaPlay's Downloader. Files are stored in the app's LocalAppData
    bin directory, so the installer stays small. Running the script again is a
    no-op when the binaries are already present. Progress output is deliberately
    generic: no URLs or local paths are printed.
#>
param(
    [Parameter(Mandatory = $true)]
    [string] $TargetDirectory,

    [switch] $IncludeFFmpeg
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
New-Item -ItemType Directory -Force -Path $TargetDirectory | Out-Null

function Save-Release {
    param([string] $Url, [string] $Destination)
    Write-Output 'Downloading official release from GitHub'
    Invoke-WebRequest -Uri $Url -OutFile $Destination -UseBasicParsing -TimeoutSec 900
    if (-not (Test-Path -LiteralPath $Destination)) { throw 'Download did not produce a file' }
}

$ytdlp = Join-Path $TargetDirectory 'yt-dlp.exe'
if (-not (Test-Path -LiteralPath $ytdlp)) {
    Save-Release -Url 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe' -Destination $ytdlp
}
Write-Output 'yt-dlp is ready'

if ($IncludeFFmpeg) {
    $ffmpeg = Join-Path $TargetDirectory 'ffmpeg.exe'
    if (-not (Test-Path -LiteralPath $ffmpeg)) {
        $archive = Join-Path $TargetDirectory 'ffmpeg-release.zip'
        Save-Release -Url 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip' -Destination $archive
        $extract = Join-Path $TargetDirectory 'ffmpeg-extract'
        if (Test-Path -LiteralPath $extract) { Remove-Item -LiteralPath $extract -Recurse -Force }
        Expand-Archive -LiteralPath $archive -DestinationPath $extract -Force
        $binDirectory = Get-ChildItem -LiteralPath $extract -Directory -Recurse |
            Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'ffmpeg.exe') } |
            Select-Object -First 1
        if (-not $binDirectory) { throw 'FFmpeg archive did not contain ffmpeg.exe' }
        Get-ChildItem -LiteralPath $binDirectory.FullName -File | Copy-Item -Destination $TargetDirectory -Force
        Remove-Item -LiteralPath $extract -Recurse -Force
        Remove-Item -LiteralPath $archive -Force
    }
    Write-Output 'FFmpeg is ready'
}

Write-Output 'TOOLS_READY'
