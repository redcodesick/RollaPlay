# Changelog

All notable changes to RollaPlay are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.3.1] - 2026-11-24

### Fixed

- Release tag verification now checks `src-tauri/Cargo.lock` and `src/version.ts` in addition to `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`, so a partially bumped tag fails before a release is created.
- The release guide now names all five synchronized version declarations and uses the GitHub remote for the example tag push.

### Notes

- Controlled updater E2E test release. No updater behavior, backend, Tauri contract, UI behavior or persisted data format changed.

## [0.3.0] - 2026-11-24

### Changed

- Polished the existing visual system without changing the palette: tightened the exact `#101010`–`#FF8C32` token set, consolidated duplicate CSS, and added shared button, skeleton, empty-state, focus, hover, pressed and disabled primitives.
- Home now leads with an honest primary action, contextual artwork when a recent file exists, real recently played thumbnails, and a clearer playlist shelf; loading states use skeletons and no content autoplays.
- Library, Recently added, playlist detail, track tables and the playback queue now have clearer hierarchy, overlap-safe thumbnails, grouped selection actions, explicit current-track state and compact row actions.
- The player bar uses a three-zone layout with centered transport/progress, a visible buffering spinner, disabled seek feedback and cancel-aware scrubbing.
- Connect, Downloader, Settings, Setup and About now show real device/tool/server states with labelled fallbacks, progress summaries and loading placeholders instead of ambiguous empty values.
- Setup, About and exit dialogs share focus-trap, Escape and backdrop-dismiss behavior; small windows keep Setup and About reachable from the bottom navigation.
- Responsive layout was verified at 1280×800, 1366×768, 1920×1080, 1024×700, 760×700 and 480×760 with no horizontal overflow or player/sidebar overlap.

### Added

- Signed in-app updates through the official Tauri updater and process plugins. About now shows installed/latest versions, status, last checked time, real GitHub release notes, byte-level progress, explicit Download & Install and Restart Now actions, and safe errors with Retry.
- Tag-triggered release workflow that verifies synchronized versions and signing secrets, builds signed NSIS/MSI artifacts, writes `latest.json` from those real artifacts and their signatures, uploads it, and verifies published assets.
- Unit tests for updater state transitions, version comparison, constrained release-note parsing, download progress, signature/manifest/network errors, retry and restart behavior.
- `docs/updater-audit.md`, `docs/updater-release-guide.md` and signing key/secret backup and rotation instructions.

### Fixed

- The compact player could place the device zone below its own bar at tablet widths; the device zone now follows the responsive grid area.
- Playlist cover play/open controls, delete confirmation, current-track artwork and thumbnail/badge collisions were corrected.
- Download progress rows now distinguish completed, failed, cancelled and indexing states, with a visible percentage.
- Media-server diagnostics and setup checks show skeleton/loading states instead of briefly reporting false "Unavailable" values.
- `prefers-reduced-motion` now suppresses transitions and animations across the app.

### Notes

- No Rust backend, SQLite schema, Axum range streaming, Cast bridge, downloader backend, Tauri contract or persisted setting was changed.
- Screenshots remain in `docs/screenshots/` for the README gallery and are not release assets.

## [0.2.0] - 2026-11-24

### Changed

- Full visual redesign with one tokenized dark palette, local Windows typography, a compact Rolla mark, and no runtime font services.
- Refined shell, sidebar, top bar, persistent playback bar, Home, Library, Playlists, Queue, Cast Connect, Downloader, Settings, Setup, About, and empty/loading states.
- Cast Connect now lists only real discovered receivers; the local-PC row is gone, the Library cast action is icon-only, and the receiver picker is a tighter dialog.
- Setup and About dialogs no longer place step counters or branding under the close button.

### Fixed

- Playlist cover artwork now opens the playlist, and its play button is no longer blocked by the artwork overlay.
- Playlist cover controls are keyboard reachable and expose clear accessible names.
- Setup guide step counters and the About brand mark no longer overlap the X.
- No backend commands, events, playback rules, Cast security model, SQLite behavior, or Downloader implementation changed.

### Added

- `docs/redesign-plan.md` with the pre-implementation audit and phase plan.
- Real-app screenshots for Home, Library, Playlist, Connect, Downloader, Setup, and About in `docs/screenshots/`.

## [0.1.1] - 2026-11-24

### Added

- Downloader tool checkboxes for **yt-dlp** and **FFmpeg** with an **Install selected with WinGet** button. The official WinGet packages are installed or upgraded on demand; no third-party binaries are bundled.
- Live status output while WinGet runs, so tool installation progress is visible in the Downloader status card.
- Safe, specific explanations for private/sign-in-only links, unavailable videos, sign-in challenges, unsupported links and outdated yt-dlp. Raw tool output is never echoed.
- Discovery of tools inside WinGet's versioned package folders, so a fresh install works without restarting the app or shell.
- Downloader tool preflight checks: RollaPlay runs `yt-dlp --version` and `ffmpeg -version` when the view opens and before every download. Missing or non-responding tools are installed automatically with WinGet (GitHub fallback), and the status pills now distinguish "ready", "not responding" and "not installed".
- Empty playlists now offer **Add videos from folder**: it opens the folder picker, indexes the chosen folders and adds the newly indexed videos directly to the open playlist.

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
- User-verified: a public YouTube download now completes after the tools are installed and the MP4 files appear in the chosen folder; the earlier silent "Downloading…" failure is gone.

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
