## RollaPlay 0.4.5

Signed Windows update. Published 2026-11-27.

### Added

- Downloader now has a **Maximum quality** selector: **Standard**, **720p max**, **1080p max** and **4K max**. Higher tiers pass real `height<=N` caps to yt-dlp, so files stay at or below the chosen resolution instead of always taking the largest source.
- Retry and Skip keep the quality chosen for the original batch, so resumed links do not change resolution.

### Changed

- Default quality is now **1080p max** instead of the previous uncapped HD selection. 4K files are only requested when you choose **4K max**.
- 4K sources that use AV1/VP9 can still be converted with the existing opt-in **TV-friendly MP4** option. FFmpeg remains manual-only; probing never installs tools.

Updates come from official GitHub Releases and are verified before installation. Nothing installs silently and your library, playlists and settings stay on this device.
