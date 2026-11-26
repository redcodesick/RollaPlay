# Changelog

All notable changes to RollaPlay are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.4.4] - 2026-11-27

### Added

- Failed Downloader rows now have **Retry** and **Skip** actions. Retry reruns that link and any unfinished links after it; Skip marks the link skipped and continues with the remaining links. A resumed batch keeps the original playlist numbering and never repeats completed, skipped or cancelled links.
- Skipped links are shown with their own phase and summary count. Library indexing waits until every original link is settled and at least one file completed, so a failed link is never silently treated as done.

## [0.4.3] - 2026-11-26

### Fixed

- Downloader status now shows the real transfer percentage, speed and sanitized video title while a download runs, including TV-friendly mode. TV-friendly mode explicitly requests yt-dlp progress because its `--print` output previously suppressed every progress line.
- The status card now labels the real phase and keeps the link position for playlist items, so it no longer stays on "Starting download" while files are transferring.

### Changed

- TV-friendly conversion now uses H.264 `preset slow` with `CRF 26` instead of `veryfast`/`CRF 20`, producing smaller MP4 files while keeping H.264/AAC MP4, `+faststart`, real stream verification and original-until-success safety.

## [0.4.2] - 2026-11-26

### Added

- RollaPlay now checks for a signed stable release at startup. An available update is shown in the app with a link to its release notes; downloading, installing and restarting still require explicit clicks.
- Tool installation now shows a real step-by-step progress bar for both WinGet and GitHub installs, with fixed milestone labels and no paths, URLs or raw script output.

### Changed

- yt-dlp and FFmpeg are no longer downloaded or installed automatically. Opening the Downloader or starting a download only probes the tools; a missing or broken tool blocks the download with guidance to use the explicit install buttons.

### Fixed

- Downloader progress now reads yt-dlp's real `[download] NN%` lines from **stderr** as well as stdout, so the status bar advances during a transfer instead of staying at 0% ([#4](https://github.com/redcodesick/RollaPlay/issues/4)). Starting and terminal phases no longer display a misleading 0% bar.

## [0.4.1] - 2026-11-26

### Fixed

- **Play → Next now follows the playlist.** "Play now" inserts the selected video immediately after the current queue entry instead of appending it at the end, so the rest of the playlist stays reachable. Playing a row that is already queued reuses its queue position instead of adding a duplicate.
- The Next button is now disabled when no playable next entry exists, instead of appearing available and then reporting "End of queue."
- At the real end of the queue, whether reached with Next or by automatic completion, the receiver is stopped and stale session ids are cleared, so the TV no longer appears to keep playing while the app says the queue has ended.
- A receiver load failure no longer marks the file unavailable and silently skips through the queue. The entry stays retryable with clear guidance to use Next or Reconnect & resume.
- Playlist playback no longer shows the misleading "Playlist added to the queue." notice. Add and Play next now have their own accurate notices.
- Cast status now prefers the receiver's live media state over a lagging `RemotePlayer` value, so a video that is playing on the TV no longer shows as Idle with an inactive seek bar in the app. The SDK's `getEstimatedTime()` and a rate-limited `getStatus()` refresh keep time correct after a manual seek. Stale or finished media is still excluded from correlation and progress.

### Added

- **Settings → Diagnostics** with an on-demand network compatibility report: private LAN adapter, local media server probe, receiver discovery, subnet classification, active receiver correlation, receiver control port, media URL probe, receiver media requests, playback state, queue continuation, and current container/MIME. It shows fixed status text and generic guidance only.
- **Downloader → Convert to TV-friendly MP4** checks the downloaded file's real audio/video streams with FFmpeg and re-encodes only unsupported files to H.264/AAC MP4 with `faststart`. The result is verified before it replaces the original; failures keep the original. Already-compatible MP4s are untouched.
- A redacted streaming-quality panel shows the selected adapter's Windows WLAN band/channel and signal/link-speed tiers, real receiver-fetch throughput and interruption tiers, and a capped in-memory log of fixed event codes. SSIDs, BSSIDs, addresses, raw output, tokens, URLs, and paths are discarded before serialization.
- Guidance covers weak 2.4 GHz headroom, channels 1/6/11, 20 MHz width, 5 GHz/Ethernet, AP isolation, and idle status while bytes flow. Channel width is reported as unavailable on Windows instead of guessed.
- The report never displays tokens, scoped media URLs, file paths, pairing keys, device addresses, raw device responses, or raw error output. Rust tests feed sensitive-looking values through the backend inputs and assert they cannot survive serialization; frontend tests cover the fixed labels, tiers, tones, and event codes.
- Guidance for the specific "control works but no video bytes" case: weak Wi-Fi/range, guest Wi-Fi, AP isolation, and 5 GHz/Ethernet are called out without exposing network details.
- WebM/MKV files whose header contains an AV1 track now produce one fixed compatibility warning in playback and in Settings → Diagnostics. AV1 WebM is still allowed to play; the report never shows codec strings or filenames.

### Notes

- Stable signed NSIS/MSI release candidate. Version 0.4.1 is a fresh version; no published 0.4.0 asset is overwritten.
- The user reported that the Mi Box black-screen symptom disappeared when the device was moved near the router. The 0.4.0 media URL check and streaming row were healthy, and the file was MP4 on the same SSID, so the remaining cause was 2.4 GHz range/throughput rather than codec or queue state.
- Existing Google Cast, DLNA/UPnP, LG webOS, RollaPlay identity, palette, fonts and signed updater behavior are preserved.

## [0.4.0] - 2026-11-25

### Added

- Real DLNA/UPnP playback for smart TVs. Connect now discovers `MediaRenderer` devices over SSDP and drives AVTransport/RenderingControl through SOAP on the local LAN; renderer-reported state feeds the existing authoritative queue engine.
- Local LG webOS SSAP control for discovered TVs: pairing, power, volume, media keys, HDMI input switching and a token-free browser fallback. Pairing keys are protected per Windows user and never displayed.
- Transport-aware UI: the Connect, Cast, player bar and home views distinguish DLNA/UPnP from Google Cast and no longer imply Chrome is required for a DLNA renderer.
- `docs/dlna-lg-validation.md` with the real-hardware gate that controls this release.

### Changed

- Existing Google Cast and signed updater flows are unchanged. A DLNA renderer uses the same short-lived, media-scoped LAN grant; completion, stop, disconnect and failed loads retire that grant.
- Poller generation and correlated session ids prevent a slow SOAP poll from reporting stale position or completing the next item. Transport controls require a live correlated DLNA session instead of replaying a finished URL.

### Notes

- Stable signed NSIS/MSI release. Version 0.4.0 is a fresh tag; no published asset from 0.3.x is overwritten.
- User-confirmed real-device validation on an LG webOS TV and a second DLNA TV was completed before this release. No mock devices, fabricated playback or fabricated completion events are used.
- Windows SmartScreen still reports the publisher as unknown because no OV/EV Authenticode code-signing certificate is configured. Updater artifacts remain minisigned and verified by the embedded public key.

## [0.3.2] - 2026-11-25

### Fixed

- Downloader tool discovery is deterministic: the app-local `bin` copy is checked first, then WinGet Links, WinGet packages and `PATH`. A stale WinGet folder can no longer shadow a working tool, and each candidate is probed in order so a broken executable does not hide a valid one (issue #3).
- "Prepare from GitHub" now refreshes existing `yt-dlp.exe` and FFmpeg through a temporary file and atomic move, so an old tool that still answers `--version` can actually be updated. The status card exposes that action as **Refresh tools from GitHub** even while the tools look ready.
- Readiness probes use an 8-second window instead of 4 seconds, and the Downloader re-probes before every download and after a failure, so the status pills no longer stay on "ready" after a real download fails.
- Download failures are classified from both stdout and stderr, with new network, stale-extractor and FFmpeg merge messages. Unmapped failures now report the yt-dlp exit code instead of the generic "Check the link and update yt-dlp" text. No raw tool output is echoed.

### Notes

- Stable signed NSIS/MSI release. The updater flow itself is unchanged from the validated 0.3.0 → 0.3.1 pass; a dedicated 0.3.1 → 0.3.2 E2E run has not been recorded yet.
- The Windows app and installer icon now uses the Rolla cat-headphones artwork. The 0.3.2 assets were rebuilt and re-signed in place because the first publication had zero downloads.
- The in-app `BrandMark` (sidebar, About, setup and profile) now uses the same cat-headphones artwork instead of the inline `R` SVG, so the app mark and the Windows icon match.

## [0.3.3] - 2026-11-25

### Changed

- The in-app `BrandMark` artwork is tightly cropped and fills its red tile instead of floating inside it. The About and profile instances were enlarged to match their containers.
- NSIS now defaults to a per-machine install under `C:\Program Files\RollaPlay` instead of the Tauri default per-user path under `%LOCALAPPDATA%`. This requires administrator approval and does **not** migrate an existing per-user install: uninstall the old copy before installing this one. The MSI already installs per-machine.

### Notes

- Version 0.3.3 is a fresh release, not an overwrite of 0.3.2: 0.3.2 had already been downloaded when these changes were finalized.

## [0.3.1] - 2026-11-24

### Fixed

- Release tag verification now checks `src-tauri/Cargo.lock` and `src/version.ts` in addition to `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`, so a partially bumped tag fails before a release is created.
- The release guide now names all five synchronized version declarations and uses the GitHub remote for the example tag push.

### Notes

- Controlled updater E2E test release. No updater behavior, backend, Tauri contract, UI behavior or persisted data format changed.
- E2E validation completed after publication: signed 0.3.0 → 0.3.1 update, restart/reopen and data preservation all passed on Windows.

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
