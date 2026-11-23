# Changelog

All notable changes to RollaPlay are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Downloader tool checkboxes for **yt-dlp** and **FFmpeg** with an **Install selected with WinGet** button. The official WinGet packages are installed or upgraded on demand; no third-party binaries are bundled.
- Live status output while WinGet runs, so tool installation progress is visible in the Downloader status card.
- Safe, specific explanations for private/sign-in-only links, unavailable videos, sign-in challenges, unsupported links and outdated yt-dlp. Raw tool output is never echoed.
- Discovery of tools inside WinGet's versioned package folders, so a fresh install works without restarting the app or shell.
- Downloader tool preflight checks: RollaPlay runs `yt-dlp --version` and `ffmpeg -version` when the view opens and before every download. Missing or non-responding tools are installed automatically with WinGet (GitHub fallback), and the status pills now distinguish "ready", "not responding" and "not installed".

### Fixed

- WinGet output is streamed line by line instead of being buffered until the process exits.
- A failed download no longer always shows the generic "check the link and update yt-dlp" text; private, unavailable and sign-in cases are recognised from more yt-dlp phrasings.
- WinGet's "already installed / no upgrade" exit code is treated as success.
- Fixed the WinGet command argument name (`installYtDlp`) so the tool installer no longer rejects the request.
- yt-dlp now receives no stdin, so it cannot hang forever on an interactive prompt in the hidden app process.
- Downloads that produce no output for five minutes are stopped with a clear stall message instead of hanging silently.
- Progress listeners stay attached when the destination folder changes, and each download row now has a real progress bar.
- A broken or stale tool executable is no longer treated as ready; WinGet's newest versioned copy is preferred and tool installation is retried when a version check fails.
- Unmapped downloader errors keep only fixed placeholders for URLs, paths, tokens and video IDs before being written to diagnostics.

## [0.1.0] - 2026-11-23

### Added

- Local media library with incremental folder scanning, Windows Shell thumbnails, search/sort, Recently added and History.
- Playlists with card/track-table views, M3U/M3U8/WPL import and export, a reorderable playback queue and watch-history clearing.
- Real `_googlecast._tcp.local.` discovery with an app-owned device menu; receiver selection opens Chrome's native Cast picker from a fully hidden embedded sender.
- Collapsible persisted sidebar, About dialog, exit confirmation and dark media-center UI using the RollaPlay design tokens.
- YouTube Downloader that saves Cast-friendly H.264/AAC MP4 files from video or playlist links, adds the folder to the library and scans it automatically. Standard quality needs no FFmpeg; HD quality fetches the official tools on first use.
- NSIS and MSI Windows x64 installers.

### Fixed

- Automatic playlist advance now requires a real `IdleReason.FINISHED` for the current media and ignores stale cached media session ids.
- Streaming URLs, bearer tokens, pairing credentials, raw folder paths and playlist lines are redacted from diagnostics.
- Cast playback no longer trusts estimated durations; seek/progress display never drives playlist advancement.
