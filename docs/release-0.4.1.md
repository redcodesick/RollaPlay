# Release verification — 0.4.1

Published on 2026-11-26 by explicit user approval. The previously pushed source commit is `6733778`; the stable tag `v0.4.1` was pushed to `github/main` and the signed release workflow completed successfully (run `45865300135`). The `v0.4.0` release and its assets remain untouched.

Release: https://github.com/redcodesick/RollaPlay/releases/tag/v0.4.1

## Why this version

`0.4.1` contains the queue/notice fixes, redacted Settings diagnostics and streaming-quality panel, Cast status/time fix, AV1 WebM compatibility warning, and opt-in TV-compatible Downloader conversion. It does not overwrite any `0.4.0` asset.

## Published signed artifacts

Built, signed, and uploaded by `.github/workflows/release.yml` from the `v0.4.1` tag. The local bundle directory now mirrors the published assets; the earlier local-only build artifacts are superseded.

- `RollaPlay_0.4.1_x64-setup.exe` — 7,203,093 bytes, SHA-256 `ee849b0c8d67e2138bd3a9c30f4a5205d1ae119e52bc5502f11fd558985396f2`
- `RollaPlay_0.4.1_x64-setup.exe.sig` — 440 bytes
- `RollaPlay_0.4.1_x64_en-US.msi` — 9,474,048 bytes, SHA-256 `9a5be542ea99f61091557d272d614167f1f37fcc547daae7c882eefbb4f28b19`
- `RollaPlay_0.4.1_x64_en-US.msi.sig` — 440 bytes

## Published manifest

`docs/release-0.4.1-latest.json` is a byte-for-byte copy of the published `latest.json`. It was regenerated from the actual published release body and the uploaded signed assets, then re-uploaded with `--clobber` so the release notes are the curated 0.4.1 notes rather than the workflow's generated full-changelog link.

- `version`: `0.4.1`
- `pub_date`: `2026-11-26T17:17:54.000Z` — taken from the real GitHub Release publication time
- `notes`: 3,882 characters, constrained Markdown, no raw HTML; matches `docs/release-notes-0.4.1.md`
- `windows-x86_64-nsis`, `windows-x86_64-msi`, and `windows-x86_64` entries are present
- Both installer signatures in the manifest match the published `.sig` assets byte-for-byte
- URLs point to the live `v0.4.1` release assets
- No screenshots are attached to the release

## Verification completed locally before tagging

- `node --check src-tauri/cast-sender/sender.js` — passed
- `node scripts/verify-release-version.mjs v0.4.1` — passed; all five version declarations are synchronized
- `pnpm test` — 19 passed across 3 files
- `pnpm build` — passed
- `cargo check --locked --manifest-path src-tauri/Cargo.toml` — passed with pre-existing warnings only
- `cargo test --locked --manifest-path src-tauri/Cargo.toml` — 114 passed, 0 failed, 1 pre-existing ignored
- `git diff --check` — passed; only CRLF normalization warnings for `Cargo.lock` and `PlayerBar.tsx`
- `rustfmt --check --config skip_children=true src-tauri/src/diagnostics.rs` — clean; repository-wide formatting still has pre-existing differences in untouched files

## Verification after publication

- Workflow run `45865300135`: all steps passed, including stable tag/version verification, signed artifact verification, release creation, manifest generation/upload, and published asset verification.
- Published release is public, not a draft and not a prerelease.
- Published `latest.json`: version `0.4.1`, RFC 3339 `pub_date`, 3,882-character notes, live NSIS/MSI URLs, and signatures matching the published `.sig` files.
- Published assets are exactly `latest.json`, the NSIS installer and signature, and the MSI installer and signature. No screenshots are attached.

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

## Update note

Windows SmartScreen will still report an unknown publisher because no OV/EV Authenticode code-signing certificate is configured. Updater artifacts remain minisigned and verified by the embedded public key.
