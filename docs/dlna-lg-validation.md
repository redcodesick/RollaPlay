# RollaPlay 0.4.0 — DLNA/UPnP and LG webOS validation gate

**Status: user-confirmed hardware validation and release authorization received on 2026-11-25.** The user confirmed that the applicable checks below passed on their own LG webOS TV and a second DLNA TV, and explicitly authorized publishing `0.4.0`. Detailed per-device observations were not supplied to the assistant, so the individual boxes remain unfilled rather than being marked with invented results.

## What changed

- A second playback transport, `src-tauri/src/dlna.rs`, discovers UPnP `MediaRenderer` devices with a real SSDP `M-SEARCH`, parses their device descriptions, and drives AVTransport/RenderingControl with SOAP. Renderer state is polled from the TV's own `GetTransportInfo`/`GetPositionInfo` responses.
- DLNA playback uses the existing LAN media server and the same short-lived, media-scoped `cast_token` grant as Google Cast. No new LAN HTTP surface, no new token, no URL in diagnostics.
- The authoritative Phase 5 queue engine is unchanged. A DLNA renderer emits the same correlated `cast-playback-event` reports as Cast; auto-advance happens only after a real `STOPPED`/`NO_MEDIA_PRESENT` state following observed `PLAYING`, with a matching playback token and session id.
- `src-tauri/src/lg_tv.rs` adds local LG webOS SSAP control: SSDP discovery, pairing over `wss://TV:3001` or `ws://TV:3000`, power/media/volume/input controls, and a token-free browser URL fallback. The pairing key is DPAPI-protected per TV and is never logged. TLS accepts only LG's published static certificate fingerprints.
- WebKit/Chrome Google Cast is preserved. Selecting a Chromecast still uses the official Web Sender SDK and Chrome's native picker.
- On older firmware that only offers `ws://TV:3000`, SSAP has no transport authentication. Pair only on a trusted LAN; webOS 23+ uses pinned `wss://TV:3001` instead.

## Why webOS playback is DLNA, not SSAP

LG's `ssap://media.viewer/open` URI hands playback to `com.webos.app.photovideo`, which is absent on webOS 23 and later. Other SSAP media-control URIs can return success even when nothing is playing. RollaPlay therefore treats SSAP as a control channel only: pairing, power, volume, HDMI input, media keys, and an optional browser URL. The actual video goes through the TV's DLNA AVTransport service.

## Test environment record (fill before claiming a pass)

- Windows edition / build:
- App commit and exact build:
- PC LAN IPv4 and interface (Wi-Fi/Ethernet):
- LG TV model, webOS version and firmware:
- LG TV LAN IPv4, wired or Wi-Fi:
- Second TV model (Samsung/other), firmware and LAN IPv4:
- Router/AP model, band, AP isolation setting:
- Sample media container, codec, resolution, duration:
- Date / tester:

## Automated checks

Run before hardware testing:

```powershell
pnpm test
pnpm build
cargo test --locked --manifest-path src-tauri/Cargo.toml
git diff --check
```

Automated coverage includes SSDP response parsing, UPnP clock/SOAP escaping, SSAP WebSocket accept validation, control endpoint mapping, token-free browser URL validation, LG client-key storage, and the existing Cast grant/event engine tests. Automated tests do **not** prove discovery, pairing, playback, or playlist completion on a real TV.

Last automated run in this workspace: `pnpm test` 12/12 passed, `pnpm build` clean, `cargo test --locked` 94 passed / 0 failed / 1 pre-existing ignored test, `git diff --check` clean apart from pre-existing LF→CRLF notices, and `node scripts/verify-release-version.mjs v0.3.3` confirmed the release-version files are still synchronized at `0.3.3`.

## LG webOS hardware checklist

- [ ] 1. **Real discovery.** With the TV on and webOS network control enabled, Connect lists the real TV name/model from `urn:lge-com:service:webos-second-screen:1` SSDP responses within a few seconds. Power the TV off or disable network control and refresh: no fabricated row remains.
- [ ] 2. **DLNA discovery.** The same TV and the Samsung/other TV appear as `DLNA` transport devices from a real SSDP `MediaRenderer` response when their DLNA renderer is enabled. No generic "TV" placeholder appears when they are off.
- [ ] 3. **First pairing.** Select the LG TV and press **PAIR WITH TV**. The TV shows its connection prompt; accepting it stores a client key. Declining it reports failure and does not claim a pairing. Reopen Connect: the TV row shows `PAIRED` without a new prompt.
- [ ] 4. **Key hygiene.** Search the app UI, logs, diagnostics, and local settings table for the returned client key value: it must never appear in plaintext. Confirm the stored value is DPAPI-protected on Windows.
- [ ] 5. **Secure transport.** On webOS 23+ the connection is `wss://TV:3001` and succeeds with LG's pinned certificate. A rogue device or unexpected certificate is rejected rather than silently trusted. Older firmware may use `ws://TV:3000`.
- [ ] 6. **TV controls.** With pairing active, verify ON, OFF, PLAY, PAUSE, STOP, VOL −, VOL +, and input switching against the physical TV. A command that the TV rejects must show an error, not success.
- [ ] 7. **Browser fallback.** Open a plain local HTTP URL (no token or query string) in the TV browser. Try a token-bearing URL and confirm RollaPlay refuses it.
- [ ] 8. **DLNA playback.** Select the LG TV as a DLNA renderer, play a supported local MP4 from the library, and confirm video appears on the TV with sound. Settings → Streaming diagnostics shows the TV's IP as the active client. Chrome is not launched and no Cast picker appears.
- [ ] 9. **Controls from the desktop.** Pause/resume, seek, volume and mute operate on the actual renderer state; the progress bar follows renderer-reported position and duration, not a frontend timer.
- [ ] 10. **Playlist progression.** Play a three-item playlist. Each item starts only after the renderer has actually played the previous file and reported STOPPED/NO_MEDIA_PRESENT with the correlated playback token. Repeat/manual modes and a stale completion from the previous item must not advance incorrectly. A naturally finished item retires its scoped grant before the next item loads.
- [ ] 11. **Stop hygiene.** Press Stop; the renderer stops, the scoped `cast_token` grant is revoked with the old URL returning 401, and no active stream remains. After natural completion, transport controls without a fresh library load report an error instead of replaying the finished URL.
- [ ] 12. **Second TV.** Repeat checks 8–11 on the Samsung/other DLNA TV. Record codec limitations honestly; do not transcode.
- [ ] 13. **Regression.** Select a Chromecast and confirm the original Google Cast flow, picker, session events, and playlist completion still work. Selecting a DLNA renderer must not leave a stale Cast session, and vice versa.
- [ ] 14. **Restart/close.** Close the app during DLNA playback. The renderer stops before the media server shuts down; reopening the app does not claim a session that is not live.

## Release gate

The user confirmed the applicable real-device checks and explicitly approved the release on 2026-11-25. Release steps:

1. Bump all five synchronized version declarations to `0.4.0` (`package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src/version.ts`).
2. Update `CHANGELOG.md` and create `docs/release-0.4.0.md` from the published run and artifacts.
3. Commit and push on a new unreleased commit, then create the stable `v0.4.0` tag.
4. Run the signed NSIS/MSI release workflow and verify `latest.json`, signatures, hashes, and notes before telling users it is available.

`v0.3.3` assets have downloads and remain untouched; `0.4.0` is a fresh tag and asset set.
