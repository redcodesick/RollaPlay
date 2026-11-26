# Release preparation — 0.4.1

Prepared locally on 2026-11-26. **Not published.** No commit, tag, push, GitHub Release, or `latest.json` upload has been made for `0.4.1`.

Tag intended for a future release: `v0.4.1`. The published `v0.4.0` release and its assets remain untouched.

## Why a new version

`0.4.0` is already published with downloaded assets. `0.4.1` is a fresh version containing the queue/notice fixes, redacted Settings diagnostics and streaming-quality panel, Cast status/time fix, AV1 WebM compatibility warning, and opt-in TV-compatible Downloader conversion. It must not overwrite any `0.4.0` asset.

## Local signed artifacts

Rebuilt with `pnpm tauri build --ci` after the final Downloader conversion change, using the protected local updater key; the signing environment variables were cleared immediately afterward. Older local `0.4.1` artifacts are stale and superseded by these files.

- `src-tauri/target/release/bundle/nsis/RollaPlay_0.4.1_x64-setup.exe` — 7,198,087 bytes, SHA-256 `dea02c0aaef0a9d5bbedb8e3f8fe72814e77c3f1221e88e082aa4149c9f4a035`
- `src-tauri/target/release/bundle/nsis/RollaPlay_0.4.1_x64-setup.exe.sig` — 440 bytes
- `src-tauri/target/release/bundle/msi/RollaPlay_0.4.1_x64_en-US.msi` — 9,474,048 bytes, SHA-256 `306e3e5aad7b892dde999d779910ff7d4bc05e49719bc923684db824c8f60cb4`
- `src-tauri/target/release/bundle/msi/RollaPlay_0.4.1_x64_en-US.msi.sig` — 440 bytes

## Local manifest candidate

`docs/release-0.4.1-latest.json` was generated from the signed local artifacts with `scripts/create-latest-json.mjs`.

- `version`: `0.4.1`
- `pub_date`: `2026-11-26T17:02:03.569Z` — this is the local preparation timestamp, not a publication timestamp
- `notes`: 3,882 characters, constrained Markdown, no raw HTML
- `windows-x86_64-nsis`, `windows-x86_64-msi`, and `windows-x86_64` entries are present
- Both installer signatures in the manifest match the local `.sig` files byte-for-byte
- URLs point to the future `v0.4.1` release assets, which do not exist yet

The release workflow must regenerate `latest.json` from the actual published release body and uploaded signed assets. This local candidate is for inspection only and must not be uploaded as-is.

## Verification completed locally

- `node --check src-tauri/cast-sender/sender.js` — passed
- `node scripts/verify-release-version.mjs v0.4.1` — passed; all five version declarations are synchronized
- `pnpm test` — 19 passed across 3 files
- `pnpm build` — passed
- `cargo check --locked --manifest-path src-tauri/Cargo.toml` — passed with pre-existing warnings only
- `cargo test --locked --manifest-path src-tauri/Cargo.toml` — 114 passed, 0 failed, 1 pre-existing ignored
- `git diff --check` — passed; only CRLF normalization warnings for `Cargo.lock` and `PlayerBar.tsx`
- `rustfmt --check --config skip_children=true src-tauri/src/diagnostics.rs` — clean; repository-wide formatting still has pre-existing differences in untouched files

## Review fixes included

- Automatic playlist completion now stops the receiver and clears stale session ids, matching manual end-of-queue behavior.
- Playlist start no longer discards warnings about unavailable or missing files when suppressing internal setup notices.
- Home-screen playlist Play now shows a success notice.
- Next is disabled when no playable queue entry exists, not merely when the queue array is non-empty.
- The diagnostics serializer now allow-lists the transport and emits fixed compatibility text instead of echoing container/MIME strings. Its adversarial test confirms sensitive-looking values do not survive serialization.
- Cast status now prefers the receiver's live media state over a lagging `RemotePlayer` value. Stale or finished media is still excluded, and the sender keeps the real `Media` object as the event target even if the SDK supplies a plain status payload.
- WebM/MKV files whose header contains an AV1 track emit one fixed compatibility warning in playback and Settings → Diagnostics. AV1 WebM remains playable and no codec string, filename, or path is exposed.
- Cast time now uses the SDK's own estimated time and a real receiver `getStatus()` refresh, including after a manual seek; no progress is fabricated with a local timer.
- Diagnostics now correlates the selected receiver with the active session, reports allow-listed WLAN and receiver-fetch tiers, and records only fixed event codes in a capped in-memory log. Windows WLAN parsing discards SSID, BSSID, MAC, adapter name, and raw command output before serialization.
- Downloader's opt-in TV-compatible mode probes the real audio/video streams, re-encodes only unsupported files to H.264/AAC MP4 with `faststart`, verifies the result, and replaces the original only after success. FFmpeg stderr and file paths are never shown or logged; failures and cancellation keep the original.

## Publication gate

Nothing has been published. A fresh explicit approval is required before committing, tagging `v0.4.1`, pushing, creating the GitHub Release, or uploading `latest.json`.

Windows SmartScreen will still report an unknown publisher because no OV/EV Authenticode code-signing certificate is configured. Updater artifacts remain minisigned and verified by the embedded public key.
