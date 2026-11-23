# RollaPlay

RollaPlay (the RockNRolla Media Station) is a local-first Tauri 2 desktop app for organizing video files on a Windows PC and streaming them with Google Cast. **Local media. Loud attitude.** The React/Vite interface is the controller; Rust owns directory access, indexing, SQLite persistence, thumbnail work, the LAN media server, and the Cast sender bridge. The browser preview intentionally does not simulate Windows filesystem access or Cast behavior.

## What you get

- **Local library:** add approved folders, scan incrementally, browse a poster grid with Windows Shell thumbnails, search, sort, and play with a single click.
- **Playlists:** create and edit playlists, import/export M3U, M3U8 and WPL, and browse Spotify/YouTube-style cards with track tables. Playing a whole playlist advances only when the receiver reports a real `IdleReason.FINISHED`.
- **Playback queue:** reorder by dragging the grip or with the arrow buttons, use `Play next in queue` on any row, and clear watch history without disturbing playlists or the queue.
- **Cast:** real `_googlecast._tcp.local.` discovery fills an app-owned device menu under the Cast pill. Selecting a receiver opens Chrome's native Cast picker from a fully hidden embedded sender. There is no floating sender window and no "This PC" receiver.
- **Polished shell:** collapsible sidebar (preference persisted), Home playlist shelf, real Recently added and History pages, an About dialog with build details, and an exit confirmation that still shuts down cleanly.

## Architecture

- **Desktop UI:** React, TypeScript, Vite, Lucide, dark media-center layout styled in `src/styles.css`.
- **Native backend:** Tauri 2 commands, SQLite in the app-local data directory (`app.windowsmediastation.local`), and Tokio-backed blocking tasks for filesystem scans and Windows Shell work.
- **Media indexing:** recursive, non-symlink-following scans of approved folders. Paths, sizes, and high-resolution modification timestamps drive incremental updates. Folder removal and missing files never delete physical media or playlist references.
- **Thumbnail engine:** Windows `IShellItemImageFactory::GetImage` with `SIIGBF_THUMBNAILONLY`. COM and Shell extraction run away from the UI thread. Generated JPEGs are in `$LOCALAPPDATA/app.windowsmediastation.local/thumbnails`; SQLite stores source metadata, cache path, dimensions, quality, status, and errors—not image bytes. Unsupported or unavailable Shell thumbnail providers yield a generic card and an actionable error.
- **Embedded HTTP media server:** Axum/Tokio binds only to a selected active RFC1918 IPv4 adapter, defaults to port 8765, and tries a fallback port range on conflicts. `GET`/`HEAD /media/{media_id}` resolves only indexed files and streams them in chunks with single-range 206 and invalid-range 416 handling; multipart ranges are deliberately rejected with 416. Canonical-root checks and capability-relative file opening protect approved directories; MIME is signature-first with a safe extension fallback. Requests require a bearer token; Windows persists it encrypted with user-scoped DPAPI. The token is revealable/copyable only through the local Tauri Settings view for deliberate pairing with a trusted device, and is never included in diagnostics or logs. Local-origin-only CORS is enabled for media requests. Status/settings are Tauri commands rather than LAN admin routes; `/health` is token-protected.
- **Cast streaming grants:** Google Cast receivers cannot send `Authorization` headers, so the media server also accepts a short-lived, media-scoped `cast_token` query credential. Grants live only in memory (12-hour sliding validity, 24-hour absolute ceiling, at most 8 live grants), authorize exactly one media id each, renew on receiver use, are revoked when the user stops playback, and never touch disk, logs, or diagnostics.
- **Google Cast sender:** the official Web Sender SDK needs Chrome, so Rust detects a Chrome installation (registry `App Paths` first, then well-known install locations), runs a loopback-only bridge on `127.0.0.1:8790–8819` serving a dedicated sender page (`src-tauri/cast-sender/`), and launches Chrome in application mode against that secure local origin. The Chrome window is re-parented into the app window as a layered, fully transparent child: no second top-level window is visible and no browser area appears in the UI. Receiver discovery uses real mDNS queries with repeated multicast/unicast probes while the list is empty. Because the Web SDK cannot select a receiver by id, choosing a device arms the selected name and injects a trusted click into Chrome's hidden `#cast-trigger` so the native picker opens with that name as guidance. No connected state is reported until a real session event arrives. The desktop app remains the primary controller: casting a library video starts the media server and sender, generates a scoped streaming URL on the current LAN IPv4, and hands only the URL, MIME type, and title to the sender. Receiver state and `RemotePlayerController` events come back through the bridge (no frontend timers). The seek bar interpolates receiver-reported position for display only and never drives playlist advancement. Closing the app asks for confirmation, then stops receiver playback, shuts down the server, detaches the sender, stops Chrome, and destroys the window in that order. Watch history can be cleared from the History page. Playback state is persisted to SQLite (`playback_state`, `playback_history`) so disconnects and restarts never lose the current media reference. Cast diagnostics in Settings never display credentials.
- **M3U/WPL playlists:** `.m3u`, `.m3u8` and `.wpl` files are scanned as playlists, not video media. Imports resolve references only to already indexed files under active roots; remote URLs are skipped, WPL UTF-16 BOMs and XML entities are handled, deleted imports are tombstoned, and exports write UTF-8 M3U/M3U8 or UTF-16LE WPL.
- **Direct play only:** no FFmpeg, transcoding, remuxing, or external thumbnail service is included. The Cast Default Media Receiver plays the original MP4/M4V/WebM/MKV files; incompatible containers are rejected before casting.

## Build and run

Prerequisites on Windows: Node.js with pnpm 12, a Rust toolchain (Cargo), WebView2 runtime, and the Visual Studio C++ build tools used by Tauri. NSIS and WiX are needed only for installers.

```powershell
pnpm install
pnpm dev                 # Vite preview at http://localhost:5173 (UI only)
pnpm build               # type check + production bundle in dist/
cargo test --manifest-path src-tauri/Cargo.toml
pnpm tauri build         # app + NSIS and MSI installers
```

Installers land in `src-tauri/target/release/bundle/nsis/RollaPlay_0.1.0_x64-setup.exe` and `src-tauri/target/release/bundle/msi/RollaPlay_0.1.0_x64_en-US.msi`. Generated folders (`node_modules`, `dist`, `src-tauri/target`, `src-tauri/gen`, `.backup`) are gitignored and never committed.

App data (SQLite database, thumbnail cache, diagnostics log) lives under `%LOCALAPPDATA%\app.windowsmediastation.local`. Setup completion is tracked locally as `wms.setup.completed`, and the sidebar preference as `wms.sidebar.collapsed`.

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

Rust tests are defined in `src-tauri/src/lib.rs`, `src-tauri/src/media_server.rs`, `src-tauri/src/network.rs`, `src-tauri/src/cast_discovery.rs`, `src-tauri/src/cast_sender.rs`, `src-tauri/src/playlist.rs`, `src-tauri/src/playlist_file.rs`, `src-tauri/src/secret.rs`, and `src-tauri/src/thumbnail.rs`. The native Windows Shell thumbnail test is ignored unless a sample video is supplied through `WMS_TEST_VIDEO`, and can be enabled on Windows with `cargo test --manifest-path src-tauri/Cargo.toml windows_shell_extracts_and_encodes_a_video_thumbnail -- --ignored`.

Cross-platform Rust unit tests cover cache safety/invalidation, migrations, scan cancellation/duplicates, rename and availability behavior, playlist preservation, M3U/WPL parsing/export and import tombstones, queue reorder and completion semantics, full/HEAD/range streaming, auth/path checks, MIME fallback, large/concurrent/interrupted streams, local CORS, port fallback, Cast streaming grants (scoping/expiry/revocation), receiver query-token authentication over HTTP, mDNS response parsing, Chrome detection/launch commands, sender asset hygiene, and Cast playback persistence.

The browser preview can verify UI layout and interactions only. Folder scanning, Tauri commands, Windows Shell extraction, drive-unplug behavior, Chrome sender behavior, real Cast discovery, and installer behavior require the native desktop app.

## License

RollaPlay is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE) (SPDX: `PolyForm-Noncommercial-1.0.0`). Use, changes, and distribution are permitted for noncommercial purposes; commercial use requires a separate agreement with the licensor.

## Validation status

On Windows the frontend build, Rust tests (77 passed, 1 ignored), and packaged NSIS/MSI builds have all been verified; recorded results are in [docs/windows-validation.md](docs/windows-validation.md). The user validated LAN Chromecast playback and receiver controls on 2026-11-23, but a follow-up run found that a finished video did not start the next one. The sender's stale media-session latch was fixed and the installers rebuilt; automatic `IdleReason.FINISHED` progression is pending re-validation on the receiver, tracked in [docs/cast-testing.md](docs/cast-testing.md). Hardware results are user-reported without per-item logs or screenshots.
