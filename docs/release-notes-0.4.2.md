## RollaPlay 0.4.2

Signed Windows update. Published 2026-11-26.

### Added

- RollaPlay now checks for a signed stable release at startup. An available update appears as an in-app notice with a link to its release notes; downloading, installing and restarting still require explicit clicks.
- Tool installation now shows a real step-by-step progress bar for both WinGet and GitHub installs, using fixed milestone labels with no paths, URLs or raw script output.

### Changed

- yt-dlp and FFmpeg are no longer downloaded or installed automatically. Opening the Downloader or starting a download only checks the tools; a missing or broken tool blocks the download with guidance to use the install buttons.

### Fixed

- Downloader progress now reads yt-dlp's real `[download] NN%` lines from stderr as well as stdout, so the bar advances during a transfer instead of staying at 0% (issue #4). Starting and terminal phases no longer show a misleading 0% bar.

Updates come from official GitHub Releases and are verified before installation. Nothing installs silently and your library, playlists and settings stay on this device.
