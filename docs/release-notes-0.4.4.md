## RollaPlay 0.4.4

Signed Windows update. Published 2026-11-27.

### Added

- Failed Downloader rows now offer **Retry** and **Skip**. Retry reruns the failed link and any unfinished links after it; Skip marks the link skipped and continues with the remaining links.
- Resumed downloads keep the original `Link N of M` positions. Completed, skipped and cancelled links are never downloaded twice, and skipped links get their own phase and summary count.

### Changed

- Library indexing now waits until every original link is settled and at least one file completed, so a failed or sign-in-only link is never silently treated as done.

Updates come from official GitHub Releases and are verified before installation. Nothing installs silently and your library, playlists and settings stay on this device.
