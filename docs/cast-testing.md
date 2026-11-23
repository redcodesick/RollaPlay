# Windows Media Station — Phase 4 Cast Testing Checklist

**Status: discovery and live session verified on a real receiver; playback/completion checklist still to run.** The workspace is Windows with Rust/Cargo, Node/pnpm and Chrome 157.0.8103.79. The packaged app discovered a Chromecast-enabled receiver named `Living Room TV` and reported `SESSION_STARTED` with that device after fixing the sender CSP (see [docs/windows-validation.md](docs/windows-validation.md)). This proves discovery and session establishment only; it does **not** prove media playback, receiving-side decoding, or `IdleReason.FINISHED` completion. Every playback item below must still be executed on real hardware before Phase 4 is considered done. Record **Pass / Fail / Blocked** with steps, expected/actual results, logs, and a screenshot where useful. A blocked test is not a pass. Do not mark any playback item complete from a code review or from this document.

## Test environment record

- Windows edition / build:
- CPU / RAM:
- App commit and version:
- Chrome version (from `chrome://version`):
- Tablet LAN IPv4 and interface (Wi-Fi/Ethernet):
- Xiaomi Mi Box model and Android TV / Cast receiver firmware version:
- Router model, band (2.4/5 GHz), and AP isolation setting:
- Sample media (container, codec, resolution, bitrate, duration):
- Date / tester:

## What the automated tests cover (already in the source tree)

Run on any machine with Rust/Cargo and the Tauri target prerequisites:
`cargo test --manifest-path src-tauri/Cargo.toml`

These tests were executed in this packaged-app validation run: 57 passed, 0 failed, 1 ignored (the Windows Shell thumbnail test needs a registered video thumbnail provider). The new `playing_a_whole_playlist_auto_advances_on_real_completion` test covers whole-playlist automatic advancement on a correlated receiver completion event.

- Cast grant store: media scoping, wrong-media rejection, empty-token rejection, revocation.
- Grant expiry: sliding 12-hour renewal, absolute 24-hour ceiling, expired-token rejection.
- Grant capacity: oldest grant evicted at the 8-grant cap.
- Cast query token parsing: single value accepted, duplicates/empty rejected, URL decoding.
- Credential hygiene: `cast_token=***` masking in diagnostics endpoints.
- Default Media Receiver container gate: mp4/m4v/webm/mkv allowed, avi/wmv/3gp/mpeg rejected.
- HTTP-level receiver auth: `GET /media/{id}?cast_token=…` without bearer returns 200, ranged `Range: bytes=2-5` returns 206 with exact `Content-Range`, other media ids return 401, duplicated query tokens return 401.
- Chrome candidate resolution: first existing executable wins, missing candidates skipped.
- Chrome launch command: `--app=http://127.0.0.1:PORT/` plus `--new-window`, `--no-first-run`, `--no-default-browser-check`.
- Sender page assets: official `cast_sender.js` framework referenced, `DEFAULT_MEDIA_RECEIVER_APP_ID`, no `cast_token` string anywhere in page/script.
- Sender event clamping: oversized/malformed strings truncated, non-numeric values dropped.
- Playback persistence: `playback_state` keeps the current media, history row records watched seconds, disconnect finalizes progress without clearing the reference, restart restores the last Cast media.

## Phase 4 hardware checklist

### Chrome sender

- [ ] 1. Chrome detection: install Chrome system-wide (and repeat with a per-user install); Settings → Cast shows "Detected" with the correct path (registry then Program Files then per-user precedence). Uninstall Chrome → the panel shows "Chrome is not installed" and the cast button in the library shows the same guidance instead of a crash.
- [ ] 2. Chrome launch: click **Launch sender**; Chrome runs in application mode on `http://127.0.0.1:<port>/` with no address bar or first-run dialog, and its window is embedded in the app's sender panel — no separate floating "Cast Sender" window. The panel stays open until a receiver is selected, then hides itself while the Cast session and app playback controls remain active.
- [ ] 3. Google Cast SDK initialization: the sender window shows "Official Cast sender SDK ready"; DevTools (if opened) shows `cast.framework` available. With the network disconnected it must show the "Cast framework could not load" error instead of hanging.

### Device discovery and session

- [ ] 4. Device discovery: with the Mi Box idle on the same network, the sender's cast state flips from "No Cast devices found" to "Devices available — not connected" within ~10 s; the device picker (sender window button or automatic) lists the Mi Box by name. No fake devices appear when the Mi Box is powered off.
- [ ] 5. Xiaomi Mi Box connection: select the Mi Box; the desktop UI pill changes to the device name, the session state becomes "Session active", and the Mi Box shows the Default Media Receiver backdrop. Disconnect from the Mi Box side (standby/power off) → the desktop UI shows the session ended state and the sender window clears the receiver name.
- [ ] 6. Reconnect: after a disconnect, cast the same or another video; the sender re-attaches without restarting Chrome.

### Media loading and streaming auth

- [ ] 7. Media URL authorization: cast an MP4; verify in Settings → Cast diagnostics that the media URL probe returns HTTP 206 with a ranged response; verify the displayed endpoint contains `cast_token=***` (never the real token) and that the pairing token never appears in any log, diagnostic, or the sender window.
- [ ] 8. MP4 playback on the Mi Box: playback starts within a few seconds; the video is fetched from the tablet over LAN (verify in Settings → Streaming diagnostics that an active stream row appears with the Mi Box IP as the client).
- [ ] 9. Unsupported format: attempt to cast an AVI/WMV/3GP file; the app shows the "cannot be played by the Cast Default Media Receiver" message and never sends it to the receiver.
- [ ] 10. Token lifetime: start a >1 hour movie; verify playback continues uninterrupted well past 12 minutes (sliding renewal) and that no authorization error appears. Wait >24 h without playback, cast again → a fresh URL is issued and playback works.
- [ ] 11. Streaming authorization expired (simulated): revoke the grant by pressing **Stop** and immediately attempting a range request from another device with the old URL → 401. Casting again issues a new working URL.

### Playback controls

- [ ] 12. Pause / resume: pause from the desktop UI and from the sender window; the Mi Box pauses; resume works from both; the desktop progress bar follows the receiver (event-driven, no timers).
- [ ] 13. Seek: drag the desktop seek slider and the sender window slider; the receiver jumps to the position; position/duration labels stay consistent with the receiver.
- [ ] 14. Volume / mute: change the desktop volume slider and mute toggle; the Mi Box volume changes audibly; the sender window slider follows receiver events; volume changes made with the Mi Box remote appear in the desktop UI.
- [ ] 15. Stop: press Stop; the Mi Box leaves the video, the streaming grant is revoked, and streaming diagnostics show no lingering active stream for that media id.
- [ ] 16. Disconnect: press Disconnect in the desktop UI; the Mi Box returns to the receiver backdrop; the library, the "last cast media" reference, and the progress are retained.

### State synchronization and resilience

- [ ] 17. Receiver disconnected mid-playback: unplug the Mi Box or leave the session; the desktop UI keeps the playlist/library state and shows the failure honestly; the sender window reports the session ended; a subsequent cast works.
- [ ] 18. Wi-Fi interruption: stream a long file and drop Wi-Fi on the tablet or the Mi Box for ~20 s; playback stalls and then fails or resumes without the app losing state; diagnostics record the interrupted stream; after the network returns, casting works again (the media server rebinds on adapter change as validated in Phase 3).
- [ ] 19. Chrome crash / sender recovery: kill `chrome.exe` from Task Manager during playback; within the disconnect grace window the app relaunches the sender window automatically (up to three attempts) while a Cast session was active; the Cast panel shows the recovery attempt counter. Close the sender window manually with nothing playing → no relaunch, status "closed".
- [ ] 20. Application restart while casting: close the Windows app during playback; the Mi Box eventually drops the session; reopen the app — the Cast panel restores "last cast media" from `playback_state` and the library/playlist data are intact.
- [ ] 21. Sender page crash (renderer): close the sender window but keep Chrome running, then cast; the app relaunches the sender page and playback proceeds.

### Security review while testing

- [ ] 22. Confirm the sender bridge listens only on `127.0.0.1` (e.g. `netstat -ano | findstr 879`): no LAN binding; a second LAN device cannot load the sender page.
- [ ] 23. Confirm no token leakage: reveal the pairing token in Settings, run a cast, and search the Cast panel diagnostics, sender window, and any logs for the token value — it must never appear. The scoped URL must never be rendered in the sender page DOM.
- [ ] 24. Confirm administrative actions (start/stop server, port changes, sender launch) are only available through the Tauri UI; the media server HTTP surface still exposes only `/media/{id}`, `/health`, and now the media-scoped `cast_token` query acceptance.

## Known limitations to document when testing

- The Default Media Receiver plays MP4/M4V/WebM/MKV containers; codec support depends on the Mi Box hardware (H.264/VP9 + AAC are safe). AVI/WMV/MPEG-PS are rejected before casting.
- Device enumeration inside the app's own list is intentionally not implemented: the official Cast Web Sender SDK does not expose a public "list receivers" API; discovery uses the framework's cast state plus Chrome's native device picker, which lists all discovered receivers by name.
- Cast discovery requires mDNS to be permitted between the PC and the Mi Box (AP isolation must be off).
