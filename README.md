# RollaPlay

RollaPlay (the RockNRolla Media Station) is a local-first Tauri 2 desktop app for organizing video files on a Windows PC and streaming them with Google Cast. **Local media. Loud attitude.** The React/Vite interface is the controller; Rust owns directory access, indexing, SQLite persistence, thumbnail work, the LAN media server, and the Cast sender bridge. The browser preview intentionally does not simulate Windows filesystem access or Cast behavior.

## Architecture

- **Desktop UI:** React, TypeScript, Vite, Lucide, dark media-center layout.
- **Native backend:** Tauri 2 commands, SQLite in the app-local data directory, and Tokio-backed blocking tasks for filesystem scans and Windows Shell work.
- **Media indexing:** recursive, non-symlink-following scans of approved folders. Paths, sizes, and high-resolution modification timestamps drive incremental updates. Folder removal and missing files never delete physical media or playlist references.
- **Thumbnail engine:** Windows `IShellItemImageFactory::GetImage` with `SIIGBF_THUMBNAILONLY`. COM and Shell extraction run away from the UI thread. Generated JPEGs are in `$LOCALAPPDATA/<app-identifier>/thumbnails`; SQLite stores source metadata, cache path, dimensions, quality, status, and errors—not image bytes. Unsupported or unavailable Shell thumbnail providers yield a generic card and an actionable error.
- **Embedded HTTP media server:** Axum/Tokio binds only to a selected active RFC1918 IPv4 adapter, defaults to port 8765, and tries a fallback port range on conflicts. `GET`/`HEAD /media/{media_id}` resolves only indexed files and streams them in chunks with single-range 206 and invalid-range 416 handling; multipart ranges are deliberately rejected with 416. Canonical-root checks and capability-relative file opening protect approved directories; MIME is signature-first with a safe extension fallback. Requests require a bearer token; Windows persists it encrypted with user-scoped DPAPI. The token is revealable/copyable only through the local Tauri Settings view for deliberate pairing with a trusted device, and is never included in diagnostics. Local-origin-only CORS is enabled for media requests. Status/settings are Tauri commands rather than LAN admin routes; `/health` is token-protected.
- **Cast streaming grants:** Google Cast receivers cannot send `Authorization` headers, so the media server also accepts a short-lived, media-scoped `cast_token` query credential. Grants live only in memory (12-hour sliding validity, 24-hour absolute ceiling, at most 8 live grants), authorize exactly one media id each, renew on receiver use, are revoked when the user stops playback, and never touch disk, logs, or diagnostics.
- **Google Cast sender (Phase 4):** the official Google Cast Web Sender SDK needs Chrome, so the Rust backend detects a Chrome installation (registry `App Paths` first, then well-known install locations), runs a loopback-only bridge on `127.0.0.1:8790–8819` that serves a dedicated sender page (`src-tauri/cast-sender/`) and streams commands over Server-Sent Events, and launches Chrome in application mode (`--app=`) against that secure local origin. Chrome's window is re-parented into the app's **Connect** overlay; there is no floating sender window and the embedded page has no playback or window controls. The device list comes from a real `_googlecast._tcp.local.` mDNS query in Rust, with an on-demand refresh. The Web SDK cannot select a receiver by id, so picking a device opens Chrome's Cast dialog with that name as guidance; no connected state is reported until a real session event arrives. The desktop app stays the primary controller: casting a library video auto-starts the media server and sender, generates the scoped streaming URL on the current LAN IPv4, and hands only the URL, MIME type, and title to the sender page. The sender reports real receiver state back through the bridge — cast/session state, device name, and media status events from `RemotePlayerController` (no frontend timers). Closing the app sends stop and disconnect before tearing down the bridge. Watch history can be cleared from Playlists & queue without touching the queue or libraries. Playback state is persisted to SQLite (`playback_state`, `playback_history`) so disconnects and app restarts never lose the current media reference. The Cast panel in Settings exposes diagnostics without ever displaying credentials.
- **M3U/WPL playlists:** `.m3u`, `.m3u8` and `.wpl` files are scanned as playlists, not video media. Imports resolve references only to already indexed files under active roots; remote URLs are skipped, WPL UTF-16 BOMs and XML entities are handled, deleted imports are tombstoned, and exports write UTF-8 M3U/M3U8 or UTF-16LE WPL.
- **Direct play only:** no FFmpeg, transcoding, remuxing, or external thumbnail service is included. The Cast Default Media Receiver plays the original MP4/M4V/WebM/MKV files; incompatible containers are rejected before casting.

## Thumbnail defaults

The default grid preview is 320 × 180 at JPEG quality 82. The library screen offers small, medium, and large presets and three quality levels. Cache entries are invalidated when indexed file size or modification timestamp changes. A refresh action retries failed extractions; cache cleanup removes orphaned and interrupted temporary files.

## Data and availability

SQLite migrations add explicit media and directory availability states:

- `available` — visible in the library.
- `temporarily_unavailable` — drive/root disappeared or could not be read; file and playlist rows are retained.
- `partially_unavailable` — a scan encountered inaccessible paths; unobserved files are not declared permanently missing.
- `permanently_missing` — a complete scan confirmed a file is absent.
- `removed_from_library` — the user removed the folder; physical files and playlist rows remain untouched.

A scan is transactional. Cancelling or interrupting it before commit discards partial index changes. Duplicate concurrent scans of the same folder are rejected. Unique path/size/modified-time matches are used to recognize likely renames while avoiding ambiguous matches.

## Development and tests

The frontend preview and production bundle use the project’s pnpm scripts. Rust tests are defined in `src-tauri/src/lib.rs`, `src-tauri/src/media_server.rs`, `src-tauri/src/network.rs`, `src-tauri/src/cast_discovery.rs`, `src-tauri/src/cast_sender.rs`, `src-tauri/src/playlist_file.rs`, `src-tauri/src/secret.rs`, and `src-tauri/src/thumbnail.rs`; run them with `cargo test --manifest-path src-tauri/Cargo.toml`. The native Windows Shell thumbnail test is ignored unless a sample video is supplied through `WMS_TEST_VIDEO`, and can be enabled on Windows with `cargo test --manifest-path src-tauri/Cargo.toml windows_shell_extracts_and_encodes_a_video_thumbnail -- --ignored`.

The native Windows Shell thumbnail provider can only be exercised on Windows.

- Cross-platform Rust unit tests cover cache safety/invalidation, migrations, scan cancellation/duplicates, rename and availability behavior, playlist preservation, M3U/WPL parsing/export and import tombstones, full/HEAD/range streaming, auth/path checks, MIME fallback, large/concurrent/interrupted streams, local CORS, port fallback, Cast streaming grants (scoping/expiry/revocation), receiver query-token authentication over HTTP, mDNS response parsing, Chrome detection/launch commands, sender asset hygiene, and Cast playback persistence.
- Windows-only manual checks are listed in [docs/windows-validation.md](docs/windows-validation.md); Phase 4 Cast hardware checks are listed in [docs/cast-testing.md](docs/cast-testing.md).
- The Linux browser preview can verify UI layout and interactions only. Folder scanning, Tauri commands, Windows Shell extraction, drive unplug behavior, Chrome sender behavior, real Cast discovery, and installer behavior require the native desktop app.

## Current phase status

Phase 4 (Google Cast) implementation is present in the source tree: scoped streaming grants, Chrome detection/launch/monitoring, the loopback sender bridge with the official Cast SDK, real receiver state display, playback controls, SQLite playback-state sync, and Cast diagnostics. The frontend build has not been verified in this Linux workspace and the Rust tests/build have not been run (no Cargo). No hardware test has been performed: real Chrome launch, Mi Box discovery, and Cast playback remain unverified until [docs/cast-testing.md](docs/cast-testing.md) is executed and recorded. Do not treat the checklists or Rust tests as passed until they are run.
