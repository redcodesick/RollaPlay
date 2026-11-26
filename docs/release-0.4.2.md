# Release verification — 0.4.2

Published on 2026-11-26 by explicit user approval so installed apps can update. The source commit is `c05cedb` (with a documentation-only follow-up at `7768f59`); the stable tag `v0.4.2` was pushed to `github/main` and the signed release workflow completed successfully (run `45886308282`). The `v0.4.0` and `v0.4.1` releases and their assets remain untouched.

Release: https://github.com/redcodesick/RollaPlay/releases/tag/v0.4.2

## Why this version

`0.4.2` adds a read-only signed-update check at startup, makes yt-dlp/FFmpeg installation manual with real step progress, and fixes Downloader progress that stayed at 0% because yt-dlp writes percentages to stderr. Nothing installs silently: download, install and restart each require an explicit click.

## Published signed artifacts

Built, signed, and uploaded by `.github/workflows/release.yml` from the `v0.4.2` tag.

- `RollaPlay_0.4.2_x64-setup.exe` — 7,219,065 bytes, SHA-256 `35fcc856c9da01ee9931575e8685ce1aa4d4ee0504bd55dc32a0d24376d8293e`
- `RollaPlay_0.4.2_x64-setup.exe.sig` — 440 bytes
- `RollaPlay_0.4.2_x64_en-US.msi` — 9,474,048 bytes, SHA-256 `10fe621eb7ca482a17ec391499397dc3268b5f550b52b04750ac3c1b09576ede`
- `RollaPlay_0.4.2_x64_en-US.msi.sig` — 440 bytes

## Published manifest

`docs/release-0.4.2-latest.json` is byte-for-byte identical to the published `latest.json`. The workflow first generated a manifest from its own release body; the release body was then replaced with the curated `docs/release-notes-0.4.2.md` and the manifest was regenerated from the real publication time and the downloaded published signatures.

- `version`: `0.4.2`
- `pub_date`: `2026-11-26T20:00:47.000Z` — the real GitHub Release publication time
- `notes`: 1,127 characters, constrained Markdown, no raw HTML; matches the published release body
- `windows-x86_64-nsis`, `windows-x86_64-msi`, and `windows-x86_64` entries are present
- Both installer signatures match the published `.sig` assets byte-for-byte
- URLs point to the live `v0.4.2` release assets
- No screenshots are attached to the release

## Verification completed locally before tagging

- `pnpm test` — 19 passed across 3 files
- `pnpm build` — passed
- `cargo check --locked --manifest-path src-tauri/Cargo.toml` — passed with pre-existing warnings only
- `cargo test --locked --manifest-path src-tauri/Cargo.toml` — 117 passed, 0 failed, 1 pre-existing ignored
- `node scripts/verify-release-version.mjs v0.4.2` — passed; all five version declarations were synchronized
- `git diff --check` — passed with only pre-existing CRLF normalization warnings

## Verification after publication

- Workflow run `45886308282`: all steps passed, including stable tag/version verification, signing-secret presence, signed artifact verification, release creation, manifest generation/upload, and published asset verification.
- Published release is public, not a draft and not a prerelease.
- Published assets are exactly `latest.json`, the NSIS installer and signature, and the MSI installer and signature. No screenshots are attached.
- Published `latest.json` matches the local copy byte-for-byte, with real notes, an RFC 3339 date, live installer URLs, and signatures matching the published `.sig` files.

## Update note

Windows SmartScreen will still report an unknown publisher because no OV/EV Authenticode code-signing certificate is configured. Updater artifacts remain minisigned and verified by the embedded public key.
