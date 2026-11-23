# RollaPlay — Phase 4 Cast Testing Checklist

**Status: discovery, live session and completion correlation fix implemented; playback/completion checklist still to run on hardware.** The workspace is Windows with Rust/Cargo, Node/pnpm and Chrome 157.0.8103.79. The packaged app discovered a Chromecast-enabled receiver named `Living Room TV` and reported `SESSION_STARTED` with that device after fixing the sender CSP (see [docs/windows-validation.md](docs/windows-validation.md)). This proves discovery and session establishment only; it does **not** prove media playback, receiving-side decoding, or `IdleReason.FINISHED` completion on hardware. The completion path was fixed after finding that a receiver assigns a new `mediaSessionId` to every load: the engine no longer keeps the previous item's ids, and the sender posts `idle_reason` on status events plus a proactive `FINISHED` check on the underlying Media object. Every playback item below must still be executed on real hardware before Phase 4 is considered done. Record **Pass / Fail / Blocked** with steps, expected/actual results, logs, and a screenshot where useful. A blocked test is not a pass. Do not mark any playback item complete from a code review or from this document.

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

These tests were executed in this packaged-app validation run: 60 passed, 0 failed, 1 ignored (the Windows Shell thumbnail test needs a registered video thumbnail provider). Whole-playlist automatic advancement is covered on a correlated receiver completion event, including a receiver changing `mediaSessionId` between items, `FINISHED` arriving only on `media_status`, and ids first arriving on `media_loaded`.

- Cast grant store: media scoping, wrong-media rejection, empty-token rejection, revocation.
- Grant expiry: sliding 12-hour renewal, absolute 24-hour ceiling, expired-token rejection.
- Grant capacity: oldest grant evicted at the 8-grant cap.
- Cast query token parsing: single value accepted, duplicates/empty rejected, URL decoding.
- Credential hygiene: `cast_token=***` masking in diagnostics endpoints.
- Default Media Receiver container gate: mp4/m4v/webm/mkv allowed, avi/wmv/3gp/mpeg rejected.
- HTTP-level receiver auth: `GET /media/{id}?cast_token=…` without bearer returns 200, ranged `Range: bytes=2-5` returns 206 with exact `Content-Range`, other media ids return 401, duplicated query tokens return 401.
- Chrome candidate resolution: first existing executable wins, missing candidates skipped.
- Chrome launch command: `--app=http://127.0.0.1:PORT/` plus `--new-window`, `--no-first-run`, `--no-default-browser-check`, `--window-size=480,340`.
- Sender page assets: official `cast_sender.js` framework referenced, `DEFAULT_MEDIA_RECEIVER_APP_ID`, no `cast_token` string anywhere in page/script.
- Sender event clamping: oversized/malformed strings truncated, non-numeric values dropped.
- Playback persistence: `playback_state` keeps the current media, history row records watched seconds, disconnect finalizes progress without clearing the reference, restart restores the last Cast media.

## Phase 4 hardware checklist

### Chrome sender

- [ ] 1. Chrome detection: install Chrome system-wide (and repeat with a per-user install); Settings → Cast shows "Detected" with the correct path (registry then Program Files then per-user precedence). Uninstall Chrome → the panel shows "Chrome is not installed" and the cast button in the library shows the same guidance instead of a crash.
- [ ] 2. Chrome launch: click the Cast pill in the top bar; a compact ROLLAPLAY device menu opens directly underneath it. Chrome runs in application mode on `http://127.0.0.1:<port>/` with no address bar or first-run dialog. Its window is embedded in a 1×1 transparent host and immediately concealed — no visible sender page, no floating sender window, no internal playback or window controls. The menu stays open until a receiver is selected, then closes itself while the Cast session and app playback controls remain active.
- [ ] 3. Google Cast SDK initialization: Settings → Cast diagnostics shows the hidden sender page connected and the framework ready; DevTools (if opened) shows `cast.framework` available. With the network disconnected the diagnostics report the "Cast framework could not load" error instead of hanging.

### Device discovery and session

- [ ] 4. Device discovery: with the receiver idle on the same LAN, Connect shows "Searching for Cast devices…" and then lists its real friendly name from `_googlecast._tcp.local.` mDNS within ~4 s. The query runs on every active LAN adapter with both multicast and unicast-response hints; **REFRESH DEVICES** issues a new live query. A restored session or a status poll must not close the menu before the list renders. No mock devices or hardcoded names appear when the receiver is powered off. If Chrome's own dialog lists a receiver that mDNS missed, it is still visible there; Connect does not invent an entry.
- [ ] 5. Xiaomi Mi Box connection: select the Mi Box; Chrome's own Cast picker opens next to the device menu — choose the same receiver there. The desktop UI pill then changes to the device name, the session state becomes "Session active", and the Mi Box shows the Default Media Receiver backdrop. Disconnect from the Mi Box side (standby/power off) → the desktop UI shows the session ended state.
- [ ] 6. Reconnect: after a disconnect, cast the same or another video; the sender re-attaches without restarting Chrome.

### Media loading and streaming auth

- [ ] 7. Media URL authorization: cast an MP4; verify in Settings → Cast diagnostics that the media URL probe returns HTTP 206 with a ranged response; verify the displayed endpoint contains `cast_token=***` (never the real token) and that the pairing token never appears in any log, diagnostic, or the sender window.
- [ ] 8. MP4 playback on the Mi Box: playback starts within a few seconds; the video is fetched from the tablet over LAN (verify in Settings → Streaming diagnostics that an active stream row appears with the Mi Box IP as the client).
- [ ] 9. Unsupported format: attempt to cast an AVI/WMV/3GP file; the app shows the "cannot be played by the Cast Default Media Receiver" message and never sends it to the receiver.
- [ ] 10. Token lifetime: start a >1 hour movie; verify playback continues uninterrupted well past 12 minutes (sliding renewal) and that no authorization error appears. Wait >24 h without playback, cast again → a fresh URL is issued and playback works.
- [ ] 11. Streaming authorization expired (simulated): revoke the grant by pressing **Stop** and immediately attempting a range request from another device with the old URL → 401. Casting again issues a new working URL.

### Playback controls

- [ ] 12. Pause / resume: pause from the desktop UI; the receiver pauses and the desktop progress bar follows the receiver (event-driven, no timers). The hidden sender has no playback controls.
- [ ] 13. Seek: drag the Cast panel seek slider; the receiver jumps to the position; position/duration labels stay consistent with the receiver. The hidden sender has no seek control.
- [ ] 14. Volume / mute: change the desktop player-bar volume slider and mute toggle; the receiver volume changes audibly; volume changes made with the receiver remote appear in the desktop UI. The hidden sender has no volume controls.
- [ ] 15. Stop: press Stop; the Mi Box leaves the video, the streaming grant is revoked, and streaming diagnostics show no lingering active stream for that media id.
- [ ] 16. Disconnect: press Disconnect in the desktop UI; the Mi Box returns to the receiver backdrop; the library, the "last cast media" reference, and the progress are retained.

### State synchronization and resilience

- [ ] 17. Receiver disconnected mid-playback: unplug the Mi Box or leave the session; the desktop UI keeps the playlist/library state and shows the failure honestly; the sender reports the session ended; a subsequent cast works.
- [ ] 18. Wi-Fi interruption: stream a long file and drop Wi-Fi on the tablet or the Mi Box for ~20 s; playback stalls and then fails or resumes without the app losing state; diagnostics record the interrupted stream; after the network returns, casting works again (the media server rebinds on adapter change as validated in Phase 3).
- [ ] 19. Chrome crash / sender recovery: kill `chrome.exe` from Task Manager during playback; within the disconnect grace window the app relaunches the sender automatically (up to three attempts) while a Cast session was active; the Cast panel shows the recovery attempt counter. Every relaunch is re-embedded into the transparent host, never a floating window; reopening the device menu resumes the session without asking for the device again. Close the sender window manually with nothing playing → no relaunch, status "closed".
- [ ] 20. Application restart while casting: close the Windows app during playback; the app sends stop and disconnect before tearing down the sender, so the Mi Box leaves the video immediately (no End Task needed); reopen the app — the Cast panel restores "last cast media" from `playback_state` and the library/playlist data are intact.
- [ ] 21. Sender page crash (renderer): close the sender window but keep Chrome running, then cast; the app relaunches the sender page into the transparent host and playback proceeds.

### Security review while testing

- [ ] 22. Confirm the sender bridge listens only on `127.0.0.1` (e.g. `netstat -ano | findstr 879`): no LAN binding; a second LAN device cannot load the sender page.
- [ ] 23. Confirm no token leakage: reveal the pairing token in Settings, run a cast, and search the Cast panel diagnostics, sender page source, and any logs for the token value — it must never appear. The scoped URL must never be rendered in the sender page DOM.
- [ ] 24. Confirm administrative actions (start/stop server, port changes, sender launch) are only available through the Tauri UI; the media server HTTP surface still exposes only `/media/{id}`, `/health`, and now the media-scoped `cast_token` query acceptance.

### Whole-playlist completion

- [ ] 25. Play a saved playlist with three short, supported files in Automatic mode. Each item must advance only after the receiver reports `IdleReason.FINISHED` for **that** load, with a changing `mediaSessionId` and a matching playback token. No timer or estimated duration may advance the queue. Repeat One, Repeat All, Manual wait, and a late `FINISHED` from the previous item must behave as documented.

## Known limitations to document when testing

- The Default Media Receiver plays MP4/M4V/WebM/MKV containers; codec support depends on the Mi Box hardware (H.264/VP9 + AAC are safe). AVI/WMV/MPEG-PS are rejected before casting.
- Connect lists real receivers from LAN mDNS, but the official Web Sender SDK has no supported API to select a receiver by id. Choosing a device opens Chrome's Cast dialog with the name shown as guidance; the actual connection is still made by that dialog, and no success is reported until a real session event arrives.
- Cast discovery requires mDNS to be permitted between the PC and the Mi Box (AP isolation must be off).
