### Fixed

- **Play → Next now follows the playlist.** "Play now" inserts the selected video immediately after the current queue entry instead of appending it at the end, so the rest of the playlist stays reachable. Playing a row that is already queued reuses its queue position instead of adding a duplicate.
- The Next button is now disabled when no playable next entry exists, instead of appearing available and then reporting "End of queue."
- At the real end of the queue, whether reached with Next or by automatic completion, the receiver is stopped and stale session ids are cleared, so the TV no longer appears to keep playing while the app says the queue has ended.
- A receiver load failure no longer marks the file unavailable and silently skips through the queue. The entry stays retryable with clear guidance to use Next or Reconnect & resume.
- Playlist playback no longer shows the misleading "Playlist added to the queue." notice. Add and Play next now have their own accurate notices.
- Cast status now prefers the receiver's live media state over a lagging `RemotePlayer` value, so a video that is playing on the TV no longer shows as Idle with an inactive seek bar in the app. The sender now uses the Cast SDK's own `getEstimatedTime()` and a rate-limited `getStatus()` refresh, so time also advances and stays correct after a manual seek. Stale or finished media is still excluded from correlation and progress.

### Added

- **Settings → Diagnostics** with an on-demand network compatibility report: private LAN adapter, local media server probe, receiver discovery, subnet classification, active receiver correlation, receiver control port, media URL probe, receiver media requests, playback state, queue continuation, and current container/MIME. It shows fixed status text and generic guidance only.
- **Downloader → Convert to TV-friendly MP4** checks the downloaded file's real audio/video streams with FFmpeg and re-encodes only unsupported files to H.264/AAC MP4 with `faststart`. The result is verified before it replaces the original; failures keep the original. Already-compatible MP4s are untouched.
- A redacted streaming-quality panel reads the selected adapter's real Windows WLAN channel, signal tier, and link-speed tier; derives a throughput tier and interrupted-fetch tier from the local media server's actual byte counts; and keeps a capped, in-memory log of fixed event codes. SSIDs, BSSIDs, MAC addresses, adapter names, raw command output, addresses, and error text are discarded before serialization.
- Guidance now covers 2.4 GHz headroom, channels 1/6/11 and 20 MHz width, 5 GHz/Ethernet, AP isolation, and the case where bytes flow while the receiver reports idle. Windows does not expose channel width here, so the panel says so instead of guessing.
- The report never displays tokens, scoped media URLs, file paths, pairing keys, device addresses, raw device responses, or raw error output. Rust tests feed sensitive-looking values through the backend inputs and assert they cannot survive serialization; frontend tests cover the fixed labels, tiers, tones, and event codes.
- WebM/MKV files whose header contains an AV1 track now produce one fixed compatibility warning in playback and in Settings → Diagnostics. AV1 WebM is still allowed to play; the report never shows codec strings or filenames.

### Notes

- Stable signed NSIS/MSI release. Version 0.4.1 is a fresh version; no published 0.4.0 asset is overwritten.
- The Mi Box black-screen symptom reported with 0.4.0 disappeared when the device was moved near the router. The media URL check and streaming row were healthy, the file was MP4 on the same SSID, and the remaining cause was 2.4 GHz range/throughput rather than codec or queue state.
- Existing Google Cast, DLNA/UPnP, LG webOS, RollaPlay identity, palette, fonts and signed updater behavior are preserved.
