## RollaPlay 0.4.3

Signed Windows update. Published 2026-11-26.

### Fixed

- Downloader now shows real per-item progress in TV-friendly mode. The `--print` step used to capture finished files previously suppressed yt-dlp progress; RollaPlay now explicitly requests progress and shows the real percentage, transfer speed, phase and video title.
- Status rows now show the sanitized video title, real phase and link position for playlists. No URLs, file paths, tokens, video IDs or raw tool output are displayed.

### Changed

- TV-friendly conversion now uses H.264 `preset slow` with `CRF 26` instead of `veryfast`/`CRF 20`, producing smaller MP4 files while keeping H.264/AAC MP4, `+faststart`, real stream verification and original-until-success safety.

Updates come from official GitHub Releases and are verified before installation. Nothing installs silently and your library, playlists and settings stay on this device.
