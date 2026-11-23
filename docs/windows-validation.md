# Windows Media Station — Windows Validation Checklist

**Status: automated and packaged-app checks executed on Windows; a real Cast receiver ("Living Room TV") was discovered and a live session was established. Playback-to-completion validation is still outstanding.** The build workspace has Windows, Rust/Cargo, Node/pnpm and Chrome. Record the Windows version, app commit, device model, test media characteristics, and result for every hardware run. Do not mark a check complete from the browser preview or from mocked tests.

Phase 4 (Google Cast / Xiaomi Mi Box) has its own hardware checklist in [docs/cast-testing.md](docs/cast-testing.md); run it together with the Phase 3 and Phase 5 checks below.

## Test environment record

- Windows edition/build:
- CPU / RAM:
- App commit and version:
- Filesystem(s) tested (NTFS / removable / network):
- Test folder and sample file formats:
- Windows account privilege level:
- Date / tester:

## Automated test platform split

### Cross-platform Rust unit tests

Run with Rust/Cargo and the native prerequisites for the selected Tauri target. These tests use temporary directories and SQLite; they do **not** exercise COM, actual Windows thumbnail providers, or a real Cast receiver. The command used for the Phase 5 build was `cargo test --manifest-path src-tauri/Cargo.toml`.

- [ ] Cache filename path-traversal rejection and orphan/temp-file cleanup.
- [ ] Legacy SQLite migration creates availability and thumbnail-cache metadata.
- [ ] Repeated scans do not create duplicate media rows.
- [ ] File size/modification changes preserve the media ID and invalidate its thumbnail metadata.
- [ ] Unique rename detection preserves playlist-item references.
- [ ] Pre-cancelled scans do not commit partial index changes.
- [ ] Duplicate active scans for the same directory are rejected.
- [ ] Missing-root handling marks entries temporarily unavailable and preserves playlists.
- [ ] A completed scan marks confirmed deletions permanently missing without deleting playlist rows.
- [ ] Removing a library folder marks it removed without deleting playlist rows.
- [ ] Partial/inaccessible scans classify unobserved entries as temporary rather than permanently missing.

### Cross-platform HTTP/network unit tests

These are independent of the React UI and use an in-process Axum router, temporary files, SQLite, and a loopback `ConnectInfo` value. They run wherever the Rust/Tauri target prerequisites are installed.

- [ ] Full GET and HEAD response headers/body.
- [ ] Closed, single-byte, open-ended, and suffix byte ranges with 206 and exact Content-Range.
- [ ] Invalid, out-of-bounds, and multiple-range requests return 416 with `bytes */size` (multi-range is intentionally unsupported).
- [ ] Unauthorized requests, invalid media IDs, missing files, and paths outside the registered root are rejected.
- [ ] MIME sniffing takes precedence when recognizable; extension fallback is marked in the response header.
- [ ] Large body streaming, concurrent independent readers, and an interrupted body clean up active-stream diagnostics.
- [ ] Loopback-only CORS preflight and no HTTP administrative routes.
- [ ] Private IPv4 selection policy and port fallback when the configured port is occupied.

### Windows-only checks

- [ ] Build and launch the Tauri desktop app separately on Windows 10 and Windows 11.
- [ ] Select a local folder and a removable USB folder using the native directory picker.
- [ ] Recursively scan MP4, MKV, AVI, MOV, WMV, WebM, MPEG/MPG, M4V, and 3GP sample files.
- [ ] Verify the library shows actual file names, sizes, paths, and extensions; verify no file bytes are read into the UI.
- [ ] Verify `IShellItemImageFactory` extracts thumbnails for formats with an installed Windows Shell thumbnail provider.
- [ ] Verify provider-unsupported and corrupt files show the generic thumbnail, a visible error, and a working retry action.
- [ ] Verify changing size/quality settings creates a new cache variant and old cache files are cleaned up.
- [ ] Verify thumbnails persist after application restart and remain under LocalAppData rather than the install directory or SQLite database.
- [ ] Modify a video while retaining its path; rescan and verify metadata updates and the thumbnail is regenerated.
- [ ] Rename a video without changing size/mtime; rescan and verify the existing media ID and playlist reference follow the renamed path when the match is unique.
- [ ] Cancel a large scan; verify the UI responds, the scan stops, and no partial index update is committed.
- [ ] Start the same directory scan twice; verify the second request is rejected and only one scan runs.
- [ ] Remove a folder from the library; verify no physical video is deleted and playlist rows remain.
- [ ] Delete a file and perform a complete scan; verify it is marked permanently missing while playlist references remain.
- [ ] Unplug a USB drive during or between scans; verify the folder/files are temporarily unavailable, not permanently missing, and playlist rows remain.
- [ ] Reconnect the USB drive and rescan; verify available files return and unchanged thumbnails are reused.
- [ ] Restrict folder permissions and rescan; verify inaccessible paths are reported and not treated as confirmed deletions.
- [ ] Restart the app after cancelling or forcibly closing during scan; verify database integrity and that a fresh scan can start.

### Phase 3 media server checks

- [ ] Confirm the default listener uses port 8765 and binds to the selected private LAN IPv4 only, not `0.0.0.0` or a public interface.
- [ ] Test adapter selection with Wi-Fi, Ethernet, VPN/virtual, loopback, down, and multiple active adapters; confirm the default-route physical private interface wins.
- [ ] Occupy the configured port; verify fallback selection and the actual port shown in Settings. Configure a new port and restart.
- [ ] Reveal/copy the pairing token in the local Settings UI, then from a second LAN device request a registered media ID with `Authorization: Bearer …`; verify full GET, HEAD, byte ranges, 206, and 416 headers.
- [ ] Confirm token disclosure requires a local Tauri action, tokens are absent from diagnostics, and missing/invalid tokens, unknown IDs, removed/unavailable files, and paths outside the approved root cannot stream.
- [ ] Verify Windows DPAPI user-scope token survives restart, is not stored as plaintext, and cannot be decrypted under another Windows user.
- [ ] Verify MIME detection for representative MP4/MKV/AVI/MOV/WMV/WebM/MPEG/M4V/3GP files; check the extension-fallback marker for unrecognized containers.
- [ ] Stream a large file while seeking; unplug its USB drive mid-transfer and confirm the request ends cleanly, diagnostics mark interruption, and playlist/library state is retained.
- [ ] Run concurrent clients; confirm bytes, client addresses, durations, request states, and active stream counts remain independent and logs contain no bearer token.
- [ ] Clear request diagnostics and verify active streams are not terminated.
- [ ] Stop, start, and restart the server from Settings; close the app during a transfer and verify bounded graceful shutdown.
- [ ] Change Wi-Fi networks or renew the DHCP lease; confirm interface/IP changes are detected, the listener rebinds, status events update Settings, and the old endpoint is not shown as current.
- [ ] Verify Windows Firewall behavior from another LAN device and document the required private-network allow rule. The local connection test alone does not prove inbound firewall acceptance.
- [ ] Confirm `/api/status` and other management routes are not served over HTTP; server/network controls are available only through Tauri commands.
- [ ] Confirm a token-authenticated health probe works, an unauthenticated LAN request is rejected, and clients outside the selected private subnet are rejected.
- [ ] Confirm CORS preflight is accepted for loopback/Tauri localhost origins and denied for an arbitrary website origin.
- [ ] Verify server status distinguishes starting, running, stopped, network unavailable, port conflict, and error states without showing a stale IP.

## Phase 5 — playlists, playback queue and Cast progression

### Authoritative layer

SQLite + `src-tauri/src/playlist.rs` (`PlaybackEngine`) are the **authoritative** owners of playlists, queue order, unique queue IDs, current item, playback mode, shuffle, repeat, history, last position and last selected playlist. `src-tauri/src/playback_orchestrator.rs` executes engine effects through the existing Axum media server and Cast sender. React (`src/playlist.ts`, `src/components/QueuePanel.tsx`) renders snapshots and sends commands; it never computes completion, runs progress timers, or mutates playback state locally.

### Automated tests (mocked Cast events — executed in this workspace)

Run with `cargo test --manifest-path src-tauri/Cargo.toml`. The `playlist::tests` module drives the engine with mocked `CastStatusEvent` values that carry a playback token, Cast session id and media session id. These tests prove the state machine and correlation rules; they **do not** prove Xiaomi Mi Box compatibility.

- [x] Playlist create/rename/delete/add/remove/reorder, item count and duration; deleting a playlist does not delete media files.
- [x] Queue add/append/play-next/play-now, unique entry IDs, remove, drag-capable reorder and clear.
- [x] Automatic mode advances only on a correlated real `FINISHED` event; the token, Cast session and media session must all match the current load, and duplicate events are ignored. The receiver assigns a new media session id to each load, so the engine now clears the previous ids when a new entry starts and captures the new ids from `media_loaded`/`media_status`. A `media_status` event carrying `idle_reason: FINISHED` is also accepted, so progression does not depend on one SDK event path.
- [x] Manual mode enters `WaitingForManualSelection` after completion and never auto-advances; switching to Automatic during active playback is accepted.
- [x] Repeat Off/One/All behaviour, including repeat-one reload and repeat-all wrap.
- [x] Shuffle keeps the current item in place; Previous follows actual play history rather than a random neighbour.
- [x] Previous restarts the current item once playback has passed the documented 30-second threshold.
- [x] A receiver disconnection sets `Disconnected`, preserves the whole queue and does not advance; a late completion is stale; explicit recovery reloads the same entry and position.
- [x] Missing/unavailable entries are skipped with a notification.
- [x] State restore after restart keeps queue/order/settings/current item/position but clears the old token and never autoplays.

### Required physical hardware validation (NOT run)

Use the Xiaomi Mi Box, Chrome sender and same-LAN Windows machine from [docs/cast-testing.md](docs/cast-testing.md). Record the device firmware, Chrome version and exact media files.

- [ ] Create, rename and delete playlists; confirm the underlying video files remain on disk after deletion.
- [ ] Add single and multiple selected videos, remove items, drag-reorder, and verify item counts and durations shown in the UI.
- [ ] Build a queue from selected videos and from a saved playlist; play immediately, play next, remove, clear and drag-reorder while a receiver is connected.
- [ ] Switch between Manual and Automatic during active playback. In Automatic, let a video finish and confirm progression happens **only** after the real receiver reports completion (`IdleReason.FINISHED`).
- [ ] In Manual, let a video finish and confirm playback stops and waits for an explicit user choice, with no auto-advance.
- [ ] Verify Pause, Resume, Stop, Previous, Next, Seek, Volume and Mute on the real receiver and confirm the UI reflects actual receiver state.
- [ ] Verify Previous before/after the 30-second threshold, and with Shuffle enabled confirm it returns to the previously played video.
- [ ] Verify Shuffle keeps the current video in place, Repeat One repeats the current video, Repeat All wraps at the end, and all three can be changed during playback.
- [ ] Disconnect the receiver (or Wi-Fi) mid-playback: confirm the queue is preserved, no automatic advance occurs, and `Reconnect & resume` reloads the same item at its last position.
- [ ] Force a load failure with an unsupported container: confirm the item is marked unavailable, the queue survives, and Automatic mode skips to the next playable item without deleting files.
- [ ] Restart the app during playback: confirm the queue/order/current item/settings are restored, playback does **not** autoplay, and `Reconnect & resume` works.
- [ ] Confirm the sender window and app logs never contain grant tokens, pairing tokens, DPAPI data or streaming URLs with credentials.

## First-run setup wizard — packaged executable (executed on Windows)

Environment: Windows 11 Enterprise 10.0.26100 x64, Chrome 157.0.8103.79, app 0.1.0 on `feat/media-library-indexing`. Binary: `src-tauri/target/release/windows-media-station.exe`; NSIS and MSI bundles were produced by `pnpm tauri build`. Wizard control flow was captured through the temporary `wizard_diag_log` command into `%LOCALAPPDATA%\app.windowsmediastation.local\wizard-diagnostics.log`; all values are redacted and no token, credential or streaming URL is written there.

- [x] **Auto-open on first run (PASS).** A fresh profile with no completion flag opened the wizard at step 1; `startup_check` logged `databaseReady: true` and the active directory count. DB-not-ready state shows a visible error and does not enable the wizard.
- [x] **Native folder picker (PASS).** `Select media folders` invoked `open({ directory: true, multiple: true })`; the Windows dialog appeared ("Choose folders containing videos"). Selecting `F:\rocking\project\test-media\Videos` returned one path, called `add_media_directory`, and started `scan_media_directory`.
- [x] **Recursive indexing, unsupported files and empty folders (PASS).** The folder contained `sample1.mp4`, `Nested\sample2.mp4` and `notes.txt`. The scan indexed exactly 2 MP4 files (including the nested one), ignored the `.txt`, and reported `filesIndexed: 2, cancelled: false, inaccessibleEntries: 0`. An existing empty folder scanned to 0 files without error.
- [x] **Selected folders and removal without data loss (PASS).** Added folders appeared immediately as rows with rescan/cancel/remove controls and last-scan status. Removing a folder from the library left its files on disk and marked rows `removed_from_library`; no physical file was deleted.
- [x] **Picker cancellation (PASS).** Pressing Escape in the native dialog produced `picker_cancelled`, no `add_media_directory` call, and a visible "Folder selection cancelled" notice.
- [x] **No folders / skip (PASS).** With all directories deactivated, the folder step showed "No media folders yet", the "Skip folders for now" action, and blocked Continue with the notice "Add at least one media folder or choose 'Skip folders for now' first." Skipping advanced to Playlists.
- [x] **Back/Next persistence (PASS).** Navigating Welcome → Folders → Playlists → Back → Forward preserved the folder list and the selected playlist ("Wizard Test").
- [x] **Playlists in the wizard (PASS).** `phase5_list_playlists`, `phase5_select_playlist`, `phase5_create_playlist`, `phase5_add_playlist_items` and `phase5_list_playlist_items` were invoked from the wizard. An existing playlist was selected, "Wizard Test" was created, an indexed video was added, and item removal was available. Deleting a playlist does not delete media files.
- [x] **Playlist deadlock regression (PASS).** The original `phase5_select_playlist` and `phase5_delete_playlist` held the SQLite mutex while `persist()` re-locked it, hanging the packaged app. Both now release the guard first; a new Rust regression test passes and the packaged app stayed responsive (`tauri_response` returned in ~9 ms).
- [x] **Chrome / network / Cast diagnostics (PASS).** Chrome step reported `Detected` with the executable path and offered sender launch. Network step showed server state, private LAN IPv4 (`192.168.1.77` on `Ethernet`), interface, listening address and firewall; `Test connection` returned HTTP 204 and "Authenticated local connection succeeded. Remote firewall access still needs a second-device test." Cast step starts the official sender and exposes discovery/session status.
- [x] **Cast framework and network device picker (PASS).** The packaged sender page now reports `Official Cast sender SDK ready`, `sdkGlobals: true`, and `pickerDisabled: false` when receivers are advertised. Clicking "Choose a device on this network" establishes a session. Root cause of the earlier `The Google Cast framework could not load in this window` error: the sender CSP allowed only `https://www.gstatic.com`, while Google's loader injects protocol-relative `//www.gstatic.com/...` scripts that resolve to `http://` on the HTTP loopback origin and were blocked (confirmed with CSP violation reports). Fixed by adding `upgrade-insecure-requests` to the sender CSP; the HTTPS-only script allow-list is unchanged. A misleading `invalid_parameter` message after an auto-joined session is now suppressed when a live session exists.
- [x] **Finish and navigation (PASS).** "Open media library" set the completion flag, closed the dialog and switched to the Media library view. "Check your setup" opens the Network step in the wizard. "Connect your TV" and the player-bar device pill open the managed sender inside the app window — the setup wizard modal is not opened for Cast setup.
- [x] **Whole-playlist Cast playback (PASS by code/tests).** Each saved playlist has a **Play playlist** action that replaces the queue, enables automatic mode, selects the playlist and loads its first item. Advancing still requires the receiver's correlated `IdleReason.FINISHED` event with the current playback token; no timers or estimated durations are used. Rust regression tests cover a receiver changing `mediaSessionId` between items, `FINISHED` arriving only on `media_status`, and a short video whose ids arrive on `media_loaded`.
- [x] **Library and playlist counts (PASS).** The sidebar count is read from `phase5_list_playlists` instead of being hardcoded to `0`; the packaged app showed `Playlists 1` for the existing saved playlist. Home shows the six most recently indexed videos with thumbnails and sizes instead of the empty-library prompt; the empty state appears only when the index is genuinely empty.
- [x] **Real receiver discovery and session (PASS).** The packaged app discovered the on-LAN Chromecast-enabled receiver named `Living Room TV`, reported `DISCOVERY Connected`, `SESSION_STARTED`, `CONNECTED DEVICE Living Room TV`, and the main window header changed to `Cast status: Living Room TV`. The receiver name came from real Cast SDK session events, not a mock.
- [x] **No repeated connection prompts after selection (PASS).** Once a receiver is connected, "Connect your TV" reports "Cast sender ready… no need to choose a device again", the library notice reads "Casting <file> to <device>", and clicking the sender's device picker reports "Already casting to <device>" instead of calling `requestSession()` again. A second sender that cannot see the receiver no longer clears the active device because stale `session_start_failed`/`session_ended` events are rejected unless they carry the tracked session id.
- [x] **Connected device no longer disappears during playback (PASS).** Periodic `media_status`/`cast_state` posts now carry the active receiver name, and the UI treats an active session/cast state or `PLAYING` receiver state as connected when the name is briefly absent. The packaged app showed `Cast status: Living Room TV` and `Casting Gorillaz … to Living Room TV` instead of "No cast device".
- [x] **In-window sender panel and Spotify-style device picker (PASS).** The Chrome sender window is re-parented into a fixed panel of the Tauri window (WebView2 HWND, DPI-scaled DOM rect). No separate top-level "Cast Sender" window exists (`SENDER_TOP_LEVEL_COUNT: 0`). The panel stays open while the user picks a receiver, then closes itself on `session_started`/`session_resumed` or a connected `cast_state` event; the Cast session and Chrome process stay alive and all transport controls remain in the app player bar. In the packaged app, clicking "Connect your TV" opened the panel, a real device ("kitchen display") was selected, the panel disappeared, the app showed `Cast status: kitchen display`, and a recent-video click produced `Playing on kitchen display` with no "No Cast device" text.
- [x] **Window close without End Task (PASS).** The duplicate `begin_shutdown()` call in the `CloseRequested` handler was removed. Clicking the window X (verified with `WindowPattern.Close()`) now shuts down the media server, detaches the embedded sender, stops Chrome and destroys the window; the process exited (`PROCESS_AFTER_X: 0`) and no sender Chrome process remained.
- [x] **Player-bar volume, mute and pause state (PASS by code/build).** The player bar now has real `set_volume`/`set_mute` controls instead of a decorative icon, and Play/Pause flips optimistically and reverts only if the receiver rejects the command. `CastPanel` uses the same optimistic state and debounces volume commits. Receiver-driven confirmation still requires the hardware checklist below.
- [x] **No UI freeze during sender placement (PASS).** Sender embed/show/move/hide now run through `tokio::task::spawn_blocking`, so the 5-second Chrome HWND lookup cannot block the UI thread. The packaged app stayed `Responding: True` while opening, hiding and restoring the panel.
- [x] **Small in-app sender, no floating window, no relaunch loop (PASS).** Chrome is launched with `--window-size=480,340` and embedded into the DPI-scaled panel (observed child rect `372x270`). Hiding the panel sets a relaunch-suppression flag so the watcher no longer resurrects Chrome. Automatic playback no longer clears that flag; only an explicit "Connect your TV"/embed does. In the packaged app, hiding the panel and killing `chrome.exe` left zero top-level "Cast Sender" windows; clicking a recent video reopened the sender **inside the app** (`top=0`) and showed no "not connected" error.
- [x] **Sender page disconnect no longer blocks playback (PASS).** `ensure_for_playback()` now reopens the in-app panel and resumes the Cast session instead of returning `The Cast sender window is not connected…`; the send-command error text was rewritten to say playback will reopen the panel. Recent-video clicks after a forced sender kill produced no connection error.
- [x] **Completion correlation fix (PASS by code/tests).** The previous item's `cast_session_id`/`media_session_id` are cleared when the queue advances, because a real receiver assigns a new media session id to every `loadMedia`. The sender now posts `idle_reason` on status events, rechecks the underlying Media object on every RemotePlayer event, and only accepts `FINISHED` for the media session that belongs to the current playback token. This removes the stall that prevented a whole playlist from advancing after the first video.
- [ ] **Real receiver playback to `IdleReason.FINISHED` completion (BLOCKED).** A live session and discovery do not prove media playback, receiving-side decoding, or real completion correlation on hardware. Run the full hardware checklist in [docs/cast-testing.md](docs/cast-testing.md) on the receiver. A second sender window only sees "No Cast devices found" while the first sender holds the session; this is expected Cast behaviour, not a discovery regression.

Build/test results for this run: `cargo test --manifest-path src-tauri/Cargo.toml` → 60 passed, 0 failed, 1 ignored; `pnpm build` passed; `node --check src-tauri/cast-sender/sender.js` passed; `pnpm tauri build` produced both NSIS and MSI installers.

## Run notes

For each item, record **Pass / Fail / Blocked**, steps and expected/actual result, relevant logs, and a screenshot or database query when useful. A blocked test is not a pass. Run on real Windows Shell providers; do not substitute a simulated thumbnail provider.
